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
mod automatic;
struct Plugin;
export!(Plugin);

mod host_adapter;
use host_adapter::{DeviceSink, HostStore, HostTransport};

fn setting(key: &str) -> Result<String, String> {
    match config::get(key) {
        Some(config::SettingValue::Text(v)) if !v.is_empty() => Ok(v),
        _ => Err(format!("Required setting {key} missing")),
    }
}
fn configured_account() -> Result<Option<u64>, String> {
    match config::get("user_id") {
        None => Ok(None),
        Some(config::SettingValue::Text(value)) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .map(Some)
            .ok_or_else(|| "Configured user_id must be a positive integer".into()),
        _ => Err("Configured user_id must be a positive integer".into()),
    }
}
fn account_id() -> Result<u64, String> {
    let configured = configured_account()?;
    let raw = HostStore.read_account_record()?;
    let raw = raw.ok_or("Withings account is not connected")?;
    let identity =
        niles_withings::application::SecureAccountRecord::authenticated_identity(&raw, now()?)
            .map_err(|_| "Account restore failed")?;
    if configured.is_some_and(|expected| expected != identity) {
        return Err("Connected account does not match configured account".into());
    }
    Ok(identity)
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
thread_local! { static OWNER: RefCell<Owner<HostStore, DeviceSink>> = RefCell::new(Owner::new()); }
fn reconcile_owner(owner: &mut Owner<HostStore, DeviceSink>) -> Result<(), String> {
    let inputs = (|| -> Result<_, String> {
        let hook = host::inbound::hook_url().ok_or("Public webhook URL unavailable")?;
        let hook = withings_client::CallbackUrl::parse(&hook).map_err(|_| "Invalid webhook URL")?;
        let client = setting("client_id")?;
        let secret = Secret::new(config::secret("client_secret").ok_or("Client secret missing")?)
            .map_err(|_| "Invalid client secret")?;
        Ok((
            hook,
            client,
            secret,
            niles_withings::lifecycle::FixedClock(now()?),
        ))
    })();
    let (hook, client, secret, clock) = match inputs {
        Ok(inputs) => inputs,
        Err(error) => {
            owner.pause();
            let _ = automatic::revoke();
            return Err(error);
        }
    };
    let status = owner.reconcile(
        &mut HostTransport,
        &mut HostStore,
        &clock,
        &client,
        &secret,
        &hook,
        "Niles Withings",
    );
    let ready = status.status == niles_withings::lifecycle::Status::Ready;
    automatic::FLEET.with(|f| {
        let mut f = f.borrow_mut();
        f.subscriptions_ready = ready;
        f.ready = ready && f.inventory_valid;
    });
    if !ready {
        automatic::revoke()?;
    }
    match status.status {
        niles_withings::lifecycle::Status::Ready => Ok(()),
        _ => Err("Account or subscription reconciliation is incomplete".into()),
    }
}
fn activate_saved() -> Result<(), String> {
    let account = account_id()?;
    OWNER.with(|o| {
        let mut owner = o.borrow_mut();
        owner
            .restore(&mut HostStore, account, now()?)
            .map_err(|e| format!("Account restore failed: {e}"))?;
        let _ = reconcile_owner(&mut owner);
        automatic::discover(&mut owner, account)?;
        let result = reconcile_owner(&mut owner);
        automatic::FLEET.with(|f| {
            let mut f = f.borrow_mut();
            f.ready = result.is_ok() && f.inventory_valid;
        });
        result
    })
}
fn refresh_panel() {
    #[cfg(not(test))]
    niles::integration::ui::invalidate("withings");
}
impl Guest for Plugin {
    fn init() -> Result<(), String> {
        // A host sensor exists only after successful inventory registration.
        // Connection/discovery failure is represented by the revoked runtime;
        // keep the plugin loaded so the host-owned Connect route remains usable.
        host::timers::every("lifecycle", 300)
            .map_err(|_| "Lifecycle timer unavailable".to_string())?;
        OWNER.with(|o| o.borrow_mut().pause());
        automatic::reset_host()?;
        if let Err(error) = activate_saved() {
            automatic::FLEET.with(|f| {
                f.borrow_mut().error = Some(if secrets::get("account").is_some() {
                    format!("Reconnect Withings or retry discovery: {error}")
                } else {
                    "Connect Withings to discover sleep mats.".into()
                })
            });
        }
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
        Ok(automatic::FLEET.with(|f| {
            let f = f.borrow();
            format!(
                "Withings: {} detected mats; delivery {}",
                f.sinks.len(),
                if f.ready { "ready" } else { "not ready" }
            )
        }))
    }
    fn set_device(_: String, _: DeviceState) -> Result<(), String> {
        Err("Read only".into())
    }
    fn on_timer(id: String) {
        if id != "lifecycle" {
            return;
        }
        OWNER.with(|o| {
            let mut owner = o.borrow_mut();
            let account = account_id();
            match account {
                Ok(account) => {
                    let _ = reconcile_owner(&mut owner);
                    let _ = automatic::discover(&mut owner, account);
                }
                Err(_) => {
                    owner.pause();
                    let _ = automatic::revoke();
                }
            }
        });
        refresh_panel();
    }
    fn on_event(_: Event) {}
    fn render(surface: String, _: RenderContext) -> Result<UiTree, String> {
        if surface != "withings" {
            return Err("Unknown surface".into());
        }
        use niles::integration::ui::{Button, ButtonStyle, Node, NodeKind, TextBlock};
        let mut labels = vec!["Withings Sleep".to_string()];
        automatic::FLEET.with(|f| {
            let f = f.borrow();
            labels.push(
                if f.account.is_some() {
                    "Withings account loaded."
                } else {
                    "Connect Withings using the host Connect control."
                }
                .into(),
            );
            labels.push(format!("{} supported sleep mats detected.", f.sinks.len()));
            labels.push(
                if f.ready {
                    "Bed-event subscriptions ready."
                } else {
                    "Bed-event delivery is not ready."
                }
                .into(),
            );
            if let Some(error) = &f.error {
                labels.push(error.clone());
            }
            for (index, sink) in f.sinks.values().enumerate() {
                let state = match *sink.occupancy.borrow() {
                    Some(true) => "In bed",
                    Some(false) => "Out of bed",
                    None => "Occupancy unknown — waiting for a fresh bed event.",
                };
                labels.push(format!("Sleep mat {}: {state}", index + 1));
            }
        });
        let mut nodes = vec![Node {
            kind: NodeKind::Stack,
            children: vec![],
        }];
        for value in labels {
            let index = nodes.len() as u32;
            nodes[0].children.push(index);
            nodes.push(Node {
                kind: NodeKind::Text(TextBlock {
                    value,
                    muted: false,
                }),
                children: vec![],
            });
        }
        let index = nodes.len() as u32;
        nodes[0].children.push(index);
        nodes.push(Node {
            kind: NodeKind::Button(Button {
                id: format!("retry:{}", OWNER.with(|o| o.borrow().generation())),
                label: "Refresh discovery".into(),
                style: ButtonStyle::Secondary,
            }),
            children: vec![],
        });
        Ok(UiTree { nodes })
    }
    fn on_action(
        surface: String,
        action_id: String,
        value: ActionValue,
        _: RenderContext,
    ) -> Result<ActionResult, String> {
        use niles::integration::ui::{ActionResult, ActionValue};
        if surface != "withings" {
            return Err("Unknown surface".into());
        }
        let mut parts = action_id.rsplitn(2, ':');
        let generation: u64 = parts
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or("Invalid setup generation")?;
        let control = parts.next().ok_or("Invalid setup action")?;
        let result = OWNER.with(|owner| {
            let mut owner = owner.borrow_mut();
            if owner.generation() != generation {
                return Err("Setup view is stale; refresh and try again".to_string());
            }
            match (control, value) {
                ("retry", ActionValue::Pressed) => {
                    let _ = reconcile_owner(&mut owner);
                    automatic::discover(&mut owner, account_id()?)?;
                    let result = reconcile_owner(&mut owner);
                    automatic::FLEET.with(|f| {
                        let mut f = f.borrow_mut();
                        f.ready = result.is_ok() && f.inventory_valid;
                    });
                    result?;
                    Ok("Sleep mats discovered automatically".to_string())
                }
                _ => Err("Unsupported setup action".to_string()),
            }
        })?;
        refresh_panel();
        Ok(ActionResult {
            toast: Some(result),
            rerender: true,
        })
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
        automatic::revoke()?;
        automatic::FLEET.with(|f| {
            let mut f = f.borrow_mut();
            f.account = None;
            f.inventory_valid = false;
        });

        let expected_account = configured_account()?;
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
                .map_err(|error| format!("Account callback failed: {error}"))
        })?;
        match outcome {
            niles_withings::application::CallbackResult::Denied => {
                return Ok("Withings access was not granted; delivery is disabled.".into());
            }
            niles_withings::application::CallbackResult::Stored { user_id }
                if expected_account.is_some_and(|expected| expected != user_id) =>
            {
                OWNER.with(|o| o.borrow_mut().pause());
                return Err(
                    "Connected account does not match configured account; delivery is disabled."
                        .into(),
                );
            }
            _ => {}
        }
        activate_saved()?;
        refresh_panel();
        Ok("Credentials saved. Supported sleep mats were discovered automatically; occupancy awaits fresh events.".into())
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
        let _ = automatic::revoke();
    }
}
