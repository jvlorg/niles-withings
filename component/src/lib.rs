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
// Fixed upstream WIT signatures exceed Clippy's argument threshold. Restrict
// the exception to generated bindings, not handwritten component code.
#[allow(clippy::too_many_arguments)]
mod bindings {
    wit_bindgen::generate!({world:"plugin",path:"wit/niles-integration.wit"});
}
use bindings::*;
mod automatic;
mod delivery_diagnostics;
mod journal_io;
mod lifecycle_diagnostics;
struct Plugin;
export!(Plugin);

mod host_adapter;
use host_adapter::{DeviceSink, HostStore, HostTransport};

fn journal(
    now: i64,
    event: &str,
    stage: &str,
    outcome: &str,
) -> Result<(), niles_withings::diagnostics::Error> {
    journal_io::write(niles_withings::diagnostics::Record {
        received_at: now,
        provider_at: None,
        mat: None,
        event: event.into(),
        stage: stage.into(),
        outcome: outcome.into(),
        correlation: journal_io::correlation(),
        count: 1,
        first_at: now,
        last_at: now,
    })
}

fn record_event(event: &str, stage: &str, outcome: &str) {
    if let Ok(timestamp) = now() {
        let _ = journal(timestamp, event, stage, outcome);
    }
}

fn setting(key: &str) -> Result<String, String> {
    match config::get(key) {
        Some(config::SettingValue::Text(v)) if !v.is_empty() => Ok(v),
        _ => Err(format!("Required setting {key} missing")),
    }
}
fn account_id() -> Result<u64, String> {
    let raw = HostStore.read_account_record()?;
    let raw = raw.ok_or("Withings account is not connected")?;
    let identity =
        niles_withings::application::SecureAccountRecord::authenticated_identity(&raw, now()?)
            .map_err(|_| "Account restore failed")?;
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
    lifecycle_diagnostics::reconciled(status);
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
        lifecycle_diagnostics::reset();
        journal_io::reset();
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
        lifecycle_diagnostics::sample("startup");
        Ok(())
    }
    fn tools() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "withings_status".into(),
            description: "Report actual Withings adapter readiness".into(),
            schema: r#"{"type":"object","properties":{}}"#.into(),
        }, ToolDef {
            name: "withings_diagnostics_export".into(),
            description: "Read a paginated, privacy-safe local Withings diagnostic journal. Does not contact Withings or change state.".into(),
            schema: r#"{"type":"object","properties":{"page":{"type":"integer","minimum":0},"per_page":{"type":"integer","minimum":1,"maximum":100},"snapshot":{"type":"string","description":"Token from page 0; required for later pages"}}}"#.into(),
        }]
    }
    fn call_tool(name: String, args: String) -> Result<String, String> {
        if name == "withings_diagnostics_export" {
            if args.len() > 1024 {
                return Err("Diagnostic arguments too large".into());
            }
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                #[serde(default)]
                page: usize,
                #[serde(default = "default_page_size")]
                per_page: usize,
                snapshot: Option<String>,
            }
            fn default_page_size() -> usize {
                25
            }
            let args: Args = serde_json::from_str(&args)
                .map_err(|_| "Invalid diagnostic arguments".to_string())?;
            if args
                .snapshot
                .as_ref()
                .is_some_and(|s| s.len() != 16 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err("Invalid diagnostic snapshot".into());
            }
            let export = niles_withings::diagnostics::export_at(
                &mut HostStore,
                now()?,
                args.page,
                args.per_page,
                args.snapshot.as_deref(),
            )
            .map_err(|e| match e {
                niles_withings::diagnostics::Error::SnapshotChanged => {
                    "Journal changed; restart export at page 0".to_string()
                }
                _ => "Diagnostic journal unavailable or invalid pagination".to_string(),
            })?;
            let mut value = serde_json::to_value(export)
                .map_err(|_| "Diagnostic export unavailable".to_string())?;
            value["evidence_scope"] =
                serde_json::json!("best_effort_withings_only_not_automation_execution");
            value["unflushed_summaries_may_be_lost_on_restart"] = serde_json::json!(true);
            let (failures, suppressed) = journal_io::pending();
            value["pending_write_failures"] = serde_json::json!(failures);
            value["pending_suppressed_requests"] = serde_json::json!(suppressed);
            if failures > 0 || suppressed > 0 {
                value["complete"] = serde_json::json!(false);
            }
            return serde_json::to_string(&value)
                .map_err(|_| "Diagnostic export unavailable".to_string());
        }
        if name != "withings_status" {
            return Err("Unknown tool".into());
        }
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
        lifecycle_diagnostics::sample("reconcile");
        if let Ok(at) = now() {
            journal_io::flush(at, true);
        }
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
        let received_at = now().ok();
        let result = (|| {
            OWNER.with(|o| o.borrow_mut().pause());
            automatic::revoke()?;
            automatic::FLEET.with(|f| {
                let mut f = f.borrow_mut();
                f.account = None;
                f.inventory_valid = false;
            });

            let cb = withings_client::CallbackUrl::parse(&callback)
                .map_err(|_| "Invalid callback".to_string())?;
            let client = setting("client_id")?;
            let secret =
                Secret::new(config::secret("client_secret").ok_or("Client secret missing")?)
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
            if matches!(outcome, niles_withings::application::CallbackResult::Denied) {
                record_event("callback", "decision", "denied");
                return Ok("Withings access was not granted; delivery is disabled.".into());
            }
            activate_saved()?;
            record_event("callback", "activation", "complete");
            refresh_panel();
            Ok("Credentials saved. Supported sleep mats were discovered automatically; occupancy awaits fresh events.".into())
        })();
        if let Some(at) = received_at {
            let _ = journal(at, "callback", "receipt", "received");
        }
        if result.is_err() {
            record_event("callback", "decision", "failed");
        }
        lifecycle_diagnostics::sample("reconcile");
        result
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
        delivery_diagnostics::begin();
        let result = OWNER.with(|o| o.borrow_mut().webhook(&input, timestamp, 86400, 300));
        delivery_diagnostics::finish(timestamp);
        refresh_panel();
        InboundResponse {
            status: result.status,
            content_type: Some("text/plain".into()),
            body: result.body,
        }
    }
    fn stop() {
        if let Ok(at) = now() {
            journal_io::flush(at, true);
        }
        OWNER.with(|o| o.borrow_mut().pause());
        let _ = automatic::revoke();
    }
}
