#![allow(clippy::too_many_arguments)]
#[cfg(not(test))]
use niles::integration as host;
#[cfg(test)]
mod test_host;
use host::{config, devices, http, secrets, storage};
use niles::integration::types;
use niles_withings::{
    adapter::{Reading, Sink},
    application::{AccountSecretStore, SelectionStore, WebhookRequest},
    coordinator::ScopedBackend,
    runtime::{AccountRecordRead, Owner},
    withings_client::{self, HttpRequest, HttpResponse, HttpTransport, Secret},
};
use std::{cell::RefCell, rc::Rc};
#[cfg(test)]
use test_host as host;
wit_bindgen::generate!({world:"plugin",path:"wit/niles-integration.wit"});
struct Plugin;
export!(Plugin);

#[derive(Clone)]
struct HostStore;
impl HostStore {
    fn get(key: &str) -> Result<Option<String>, String> {
        Ok(storage::get(key))
    }
    fn set(key: &str, value: &str) -> Result<(), String> {
        storage::set(key, value).map_err(|_| "Host storage write failed".to_string())
    }
}
impl AccountSecretStore for HostStore {
    type Error = String;
    fn set_account_record(&mut self, value: &str) -> Result<(), String> {
        secrets::set("account", value).map_err(|_| "Encrypted account write failed".into())
    }
}
impl AccountRecordRead for HostStore {
    fn read_account_record(&mut self) -> Result<Option<String>, String> {
        secrets::get("account")
            .ok_or_else(|| "Encrypted account record missing".to_string())
            .map(Some)
    }
}

impl SelectionStore for HostStore {
    type Error = String;
    fn load_selection(&mut self) -> Result<Option<String>, String> {
        Self::get("selection")
    }
    fn save_selection(&mut self, value: &str) -> Result<(), String> {
        Self::set("selection", value)
    }
}
impl ScopedBackend for HostStore {
    type Error = String;
    fn load_value(&mut self, key: &str) -> Result<Option<String>, String> {
        Self::get(key)
    }
    fn save_value(&mut self, key: &str, value: &str) -> Result<(), String> {
        Self::set(key, value)
    }
}
#[derive(Clone, Default)]
struct DeviceSink {
    id: Rc<RefCell<Option<String>>>,
    occupancy: Rc<RefCell<Option<bool>>>,
}
impl DeviceSink {
    fn target(&self) -> Result<String, String> {
        self.id
            .borrow()
            .clone()
            .ok_or_else(|| "Registered Withings device is unavailable".into())
    }
    fn set_target(&self, id: String) {
        *self.id.borrow_mut() = Some(id);
    }
}
impl Sink for DeviceSink {
    type Error = String;
    fn publish_reading(&mut self, reading: Reading) -> Result<(), String> {
        let id = self.target()?;
        *self.occupancy.borrow_mut() = None;
        let occupied = match reading {
            Reading::Occupied { occupied, .. } => Some(occupied),
            _ => None,
        };
        let result = match reading {
            Reading::Unknown { .. } => devices::set_available(&id, false)
                .map_err(|_| "Could not mark device unavailable".into()),
            Reading::Occupied {
                occupied,
                observed_at,
            } => devices::report_state(
                &id,
                &types::DeviceState {
                    on: None,
                    brightness: None,
                    color_temp_kelvin: None,
                    rgb: None,
                    temperature_celsius: None,
                    humidity_percent: None,
                    battery_percent: None,
                    open: None,
                    motion: None,
                    illuminance_lux: None,
                    position: None,
                    readings: vec![
                        types::Reading {
                            name: "in_bed".into(),
                            value: if occupied { 1.0 } else { 0.0 },
                            unit: None,
                        },
                        types::Reading {
                            name: "observed_at".into(),
                            value: observed_at as f64,
                            unit: Some("unix_seconds".into()),
                        },
                    ],
                },
            )
            .map_err(|_| "Could not publish Withings reading".to_string())
            .and_then(|()| {
                devices::set_available(&id, true)
                    .map_err(|_| "Could not confirm availability".to_string())
            }),
        };
        if result.is_ok() {
            *self.occupancy.borrow_mut() = occupied;
        }
        result
    }
    fn publish_action(&mut self, action: &'static str) -> Result<(), String> {
        devices::report_action(&self.target()?, action)
            .map_err(|_| "Could not publish Withings action".into())
    }
}
struct HostTransport;
impl HttpTransport for HostTransport {
    type Error = String;
    fn post(&mut self, r: HttpRequest) -> Result<HttpResponse, String> {
        let mut headers = vec![(
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        )];
        if let Some(t) = r.bearer {
            headers.push((
                "authorization".into(),
                format!("Bearer {}", t.expose_secret()),
            ));
        }
        let response = http::send(&http::Request {
            method: http::Method::Post,
            url: r.url.into(),
            headers,
            body: Some(r.form.into_bytes()),
            timeout_ms: Some(5000),
        })
        .map_err(|_| "Provider request failed".to_string())?;
        Ok(HttpResponse {
            status: response.status,
            body: response.body,
        })
    }
}

fn setting(key: &str) -> Result<String, String> {
    match config::get(key) {
        Some(config::SettingValue::Text(v)) if !v.is_empty() => Ok(v),
        _ => Err(format!("Required setting {key} missing")),
    }
}
fn account_id() -> Result<u64, String> {
    setting("user_id")?
        .parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "Configured user_id must be a positive integer".into())
}
#[cfg(test)]
fn now() -> Result<i64, String> {
    test_host::clock()
}
#[cfg(not(test))]
fn now() -> Result<i64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| "Clock unavailable".into())
}
thread_local! { static OWNER: RefCell<Owner<HostStore, DeviceSink>> = RefCell::new(Owner::new()); static SINK: DeviceSink = DeviceSink::default(); }
fn reconcile_owner(owner: &mut Owner<HostStore, DeviceSink>) -> Result<(), String> {
    let hook = host::inbound::hook_url().ok_or("Public webhook URL unavailable")?;
    let hook = withings_client::CallbackUrl::parse(&hook).map_err(|_| "Invalid webhook URL")?;
    let client = setting("client_id")?;
    let secret = Secret::new(config::secret("client_secret").ok_or("Client secret missing")?)
        .map_err(|_| "Invalid client secret")?;
    let clock = niles_withings::lifecycle::FixedClock(now()?);
    let status = owner.reconcile(
        &mut HostTransport,
        &mut HostStore,
        &clock,
        &client,
        &secret,
        &hook,
        "Niles Withings",
    );
    match status.status {
        niles_withings::lifecycle::Status::Ready => Ok(()),
        _ => Err("Account or subscription reconciliation is incomplete".into()),
    }
}
fn activate_saved() -> Result<(), String> {
    let account = account_id()?;
    let mut owner = OWNER.with(|o| std::mem::replace(&mut *o.borrow_mut(), Owner::new()));
    let result = (|| {
        let mut store = HostStore;
        owner
            .restore(&mut store, account, now()?)
            .map_err(|_| "Account restore failed".to_string())?;
        reconcile_owner(&mut owner)?;
        let mut transport = HostTransport;
        owner
            .discover(&mut transport, &mut store, now()?)
            .map_err(|_| "Device discovery failed".to_string())?;
        let configured = setting("device_id")?;
        let sink = SINK.with(Clone::clone);
        if owner
            .presentation()
            .selected
            .as_ref()
            .is_some_and(|c| c.action_id() == configured)
        {
            sink.set_target(format!("{}/withings_sleep", setting("room")?));
            owner
                .activate(&mut store, account, HostStore, sink)
                .map_err(|_| "Saved selection activation failed".to_string())?;
        } else {
            owner.switch_mat();
            owner
                .discover(&mut transport, &mut store, now()?)
                .map_err(|_| "Replacement discovery failed".to_string())?;
            if let Some(candidate) = owner
                .presentation()
                .candidates
                .iter()
                .find(|c| c.action_id() == configured)
            {
                sink.set_target(format!("{}/withings_sleep", setting("room")?));
                owner
                    .accept(
                        &mut store,
                        candidate.action_id(),
                        owner.generation(),
                        HostStore,
                        sink,
                    )
                    .map_err(|_| "Configured device selection failed".to_string())?;
            } else {
                return Err("Configured mat is absent from eligible inventory".into());
            }
        }
        reconcile_owner(&mut owner)?;
        Ok(())
    })();
    OWNER.with(|o| *o.borrow_mut() = owner);
    result
}
fn refresh_panel() {
    #[cfg(not(test))]
    niles::integration::ui::invalidate("withings");
}
impl Guest for Plugin {
    fn init() -> Result<(), String> {
        let id = format!("{}/withings_sleep", setting("room")?);
        devices::upsert(&types::Device {
            id: id.clone(),
            class: types::DeviceClass::Sensor,
            light: types::LightCapabilities {
                color_temp: false,
                rgb: false,
            },
            controllable: false,
            state: types::DeviceState {
                on: None,
                brightness: None,
                color_temp_kelvin: None,
                rgb: None,
                temperature_celsius: None,
                humidity_percent: None,
                battery_percent: None,
                open: None,
                motion: None,
                illuminance_lux: None,
                position: None,
                readings: vec![],
            },
        })
        .map_err(|_| "Device registration failed".to_string())?;
        SINK.with(|s| s.set_target(id.clone()));
        devices::set_available(&id, false).map_err(|_| "Device availability failed".to_string())?;
        // Connection/discovery failure is represented by the revoked runtime;
        // keep the plugin loaded so the host-owned Connect route remains usable.
        host::timers::every("lifecycle", 300)
            .map_err(|_| "Lifecycle timer unavailable".to_string())?;
        let _ = activate_saved();
        Ok(())
    }
    fn tools() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "withings_status".into(),
            description: "Report actual Withings adapter readiness".into(),
            schema: r#"{"type":"object","properties":{}}"#.into(),
        }]
    }
    fn call_tool(_: String, _: String) -> Result<String, String> {
        Ok(OWNER.with(|o| format!("Withings runtime: {:?}", o.borrow().presentation())))
    }
    fn set_device(_: String, _: DeviceState) -> Result<(), String> {
        Err("Read only".into())
    }
    fn on_timer(id: String) {
        if id != "lifecycle" {
            return;
        }
        OWNER.with(|o| {
            if reconcile_owner(&mut o.borrow_mut()).is_err() {
                SINK.with(|s| {
                    if let Ok(id) = s.target() {
                        *s.occupancy.borrow_mut() = None;
                        let _ = devices::set_available(&id, false);
                    }
                });
            }
        });
        refresh_panel();
    }
    fn on_event(_: Event) {}
    fn render(surface: String, _: RenderContext) -> Result<UiTree, String> {
        if surface != "withings" {
            return Err("Unknown surface".into());
        }
        use niles::integration::ui::{Node, NodeKind, TextBlock};
        use niles_withings::coordinator::{DiscoveryState, SubscriptionState};
        let view = OWNER.with(|o| o.borrow().presentation());
        let account = match view.discovery {
            DiscoveryState::AuthorizationRequired | DiscoveryState::InterruptedRefresh => {
                "Reconnect Withings using the host Connect control."
            }
            DiscoveryState::NotStarted => "Connect Withings using the host Connect control.",
            _ => "Withings account loaded.",
        };
        let mat = match view.discovery {
            DiscoveryState::Selected => "Configured sleep mat selected.",
            DiscoveryState::NoEligibleMats => "No supported sleep mat found in this account.",
            DiscoveryState::StaleSelection => {
                "Saved sleep mat is no longer available. No replacement was selected."
            }
            DiscoveryState::Unavailable => "Sleep mat discovery is unavailable.",
            DiscoveryState::Candidate | DiscoveryState::MultipleCandidates => {
                "Set the intended device ID in add-on settings; no mat is selected yet."
            }
            _ => "Sleep mat not active.",
        };
        let subscriptions = match view.subscription {
            SubscriptionState::Ready => "Bed-event subscriptions ready.",
            SubscriptionState::Incomplete => "Bed-event subscriptions incomplete.",
            SubscriptionState::NotConfigured => "Bed-event subscriptions not configured.",
        };
        let mut nodes = vec![Node {
            kind: NodeKind::Stack,
            children: vec![1, 2, 3, 4, 5],
        }];
        let occupancy = SINK.with(|s| match *s.occupancy.borrow() {
            Some(true) => "In bed",
            Some(false) => "Out of bed",
            None => "Occupancy unknown — waiting for a fresh bed event.",
        });
        for value in ["Withings Sleep", account, mat, subscriptions, occupancy] {
            nodes.push(Node {
                kind: NodeKind::Text(TextBlock {
                    value: value.into(),
                    muted: false,
                }),
                children: vec![],
            });
        }
        Ok(UiTree { nodes })
    }
    fn on_action(
        _: String,
        _: String,
        _: ActionValue,
        _: RenderContext,
    ) -> Result<ActionResult, String> {
        Err("Setup selection UI remains incomplete".into())
    }
    fn on_mqtt_message(_: u32, _: String, _: Vec<u8>) {}
    fn on_mqtt_status(_: u32, _: ConnectionStatus) {}
    fn authorize_url(callback: String, state: String) -> Result<String, String> {
        let client = setting("client_id")?;
        let state = Secret::new(state).map_err(|_| "Invalid state".to_string())?;
        withings_client::authorization_url(&client, &callback, &state)
            .map_err(|_| "Invalid authorization configuration".into())
    }
    fn on_callback(
        callback: String,
        query: Vec<(String, String)>,
        person: Option<String>,
    ) -> Result<String, String> {
        OWNER.with(|o| o.borrow_mut().pause());
        SINK.with(|s| *s.occupancy.borrow_mut() = None);
        SINK.with(|s| {
            devices::set_available(&s.target()?, false)
                .map_err(|_| "Could not revoke availability".to_string())
        })?;
        let cb = withings_client::CallbackUrl::parse(&callback)
            .map_err(|_| "Invalid callback".to_string())?;
        let client = setting("client_id")?;
        let secret = Secret::new(config::secret("client_secret").ok_or("Client secret missing")?)
            .map_err(|_| "Invalid client secret")?;
        let outcome = OWNER.with(|o| {
            o.borrow_mut()
                .callback(
                    &mut HostTransport,
                    &mut HostStore,
                    &cb,
                    &query,
                    person.as_deref(),
                    &client,
                    &secret,
                    now()?,
                )
                .map_err(|_| "Account callback failed".to_string())
        })?;
        match outcome {
            niles_withings::application::CallbackResult::Denied => {
                return Ok("Withings access was not granted; delivery is disabled.".into());
            }
            niles_withings::application::CallbackResult::Stored { user_id }
                if user_id != account_id()? =>
            {
                return Err(
                    "Connected account does not match configured account; delivery is disabled."
                        .into(),
                );
            }
            _ => {}
        }
        activate_saved()?;
        Ok("Credentials saved, configured mat activated and subscriptions reconciled. Occupancy remains unknown until a fresh event.".into())
    }
    fn on_webhook(request: InboundRequest) -> InboundResponse {
        let content_type = request
            .headers
            .iter()
            .find(|(k, _)| k == "content-type")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let input = WebhookRequest {
            method: request.method,
            content_type,
            body: request.body,
        };
        let timestamp = match now() {
            Ok(timestamp) => timestamp,
            Err(_) => {
                return InboundResponse {
                    status: 503,
                    content_type: Some("text/plain".into()),
                    body: b"Clock unavailable".to_vec(),
                };
            }
        };
        let result = OWNER.with(|o| o.borrow_mut().webhook(&input, timestamp, 86400, 300));
        refresh_panel();
        InboundResponse {
            status: result.status,
            content_type: Some("text/plain".into()),
            body: result.body,
        }
    }
    fn stop() {
        OWNER.with(|o| o.borrow_mut().pause());
        SINK.with(|s| *s.occupancy.borrow_mut() = None);
    }
}
