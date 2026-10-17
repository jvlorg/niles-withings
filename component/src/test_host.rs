//! Synthetic imports: execute the real component entry points without a network or Niles host.
use super::*;
use std::collections::BTreeMap;
#[derive(Default)]
struct Ledger {
    secrets: BTreeMap<String, String>,
    storage: BTreeMap<String, String>,
    registered: Vec<String>,
    available: BTreeMap<String, bool>,
    actions: Vec<(String, String)>,
    readings: Vec<(String, f64)>,
    requests: Vec<String>,
    timers: Vec<String>,
    time: i64,
    fail_final: bool,
    wrong_mat: bool,
    fail_host_write: bool,
    fail_registration: bool,
    fail_availability: bool,
    discovery_error: bool,
    empty_inventory: bool,
    multiple_mats: bool,
    legacy_id: bool,
    configured_user: Option<String>,
    second_account: bool,
    subscription_error: bool,
}
thread_local! { static LEDGER: RefCell<Ledger> = RefCell::new(Ledger::default()); }
pub fn clock() -> Result<i64, String> {
    Ok(LEDGER.with(|l| l.borrow().time))
}
pub mod config {
    pub use crate::niles::integration::config::SettingValue;
    pub fn get(k: &str) -> Option<SettingValue> {
        Some(SettingValue::Text(
            match k {
                "device_id" if super::LEDGER.with(|l| l.borrow().legacy_id) => "offline-mat",
                "room" => "bedroom",
                "user_id" => {
                    return super::LEDGER
                        .with(|l| l.borrow().configured_user.clone())
                        .map(SettingValue::Text);
                }
                "client_id" => "fixture-client",
                _ => return None,
            }
            .into(),
        ))
    }
    pub fn secret(_: &str) -> Option<String> {
        Some("fixture-secret".into())
    }
}
pub mod secrets {
    use super::*;
    pub fn get(k: &str) -> Option<String> {
        LEDGER.with(|l| l.borrow().secrets.get(k).cloned())
    }
    pub fn set(k: &str, v: &str) -> Result<(), ()> {
        LEDGER.with(|l| {
            let mut l = l.borrow_mut();
            if l.fail_final && !v.contains("\"refresh_pending\":true") {
                return Err(());
            }
            l.secrets.insert(k.into(), v.into());
            Ok(())
        })
    }
}
pub mod storage {
    use super::*;
    pub fn get(k: &str) -> Option<String> {
        LEDGER.with(|l| l.borrow().storage.get(k).cloned())
    }
    pub fn set(k: &str, v: &str) -> Result<(), ()> {
        LEDGER.with(|l| -> Result<(), ()> {
            let mut l = l.borrow_mut();
            if l.fail_host_write && k == "host_devices" {
                return Err(());
            }
            l.storage.insert(k.into(), v.into());
            Ok(())
        })
    }
}
pub mod devices {
    use super::*;
    pub fn upsert(d: &types::Device) -> Result<(), ()> {
        if LEDGER.with(|l| l.borrow().fail_registration) {
            return Err(());
        }
        LEDGER.with(|l| l.borrow_mut().registered.push(d.id.clone()));
        Ok(())
    }
    pub fn set_available(id: &str, value: bool) -> Result<(), ()> {
        if LEDGER.with(|l| l.borrow().fail_availability) {
            return Err(());
        }
        LEDGER.with(|l| {
            let mut l = l.borrow_mut();
            assert!(l.registered.iter().any(|x| x == id));
            l.available.insert(id.into(), value);
        });
        Ok(())
    }
    pub fn report_state(id: &str, s: &types::DeviceState) -> Result<(), ()> {
        LEDGER.with(|l| {
            let mut l = l.borrow_mut();
            assert!(l.registered.iter().any(|x| x == id));
            for r in &s.readings {
                l.readings.push((r.name.clone(), r.value));
            }
        });
        Ok(())
    }
    pub fn report_action(id: &str, a: &str) -> Result<(), ()> {
        LEDGER.with(|l| l.borrow_mut().actions.push((id.into(), a.into())));
        Ok(())
    }
}
pub mod timers {
    pub fn every(id: &str, _: u32) -> Result<(), ()> {
        super::LEDGER.with(|l| l.borrow_mut().timers.push(id.into()));
        Ok(())
    }
}
pub mod inbound {
    pub fn hook_url() -> Option<String> {
        Some("https://example.org/hook".into())
    }
}
pub mod http {
    use super::*;
    pub use crate::niles::integration::http::{Method, Request, Response};
    pub fn send(r: &Request) -> Result<Response, ()> {
        assert!(matches!(r.method, Method::Post));
        assert_eq!(r.timeout_ms, Some(5000));
        let body = std::str::from_utf8(r.body.as_ref().unwrap()).unwrap();
        if LEDGER.with(|l| l.borrow().discovery_error) && body == "action=getdevice" {
            return Err(());
        }
        LEDGER.with(|l| l.borrow_mut().requests.push(body.into()));
        let response = if body.contains("grant_type=") {
            r#"{"status":0,"body":{"userid":123,"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600,"client_id":"fixture-client"}}"#.to_string()
        } else if body.starts_with("action=list") {
            if LEDGER.with(|l| l.borrow().subscription_error) {
                return Err(());
            }
            assert!(
                r.headers
                    .iter()
                    .any(|(k, v)| k == "authorization" && v == "Bearer fixture-access")
            );
            r#"{"status":0,"body":{"profiles":[{"appli":50,"callbackurl":"https://example.org/hook","comment":"Niles Withings"},{"appli":51,"callbackurl":"https://example.org/hook","comment":"Niles Withings"}]}}"#.to_string()
        } else if body == "action=getdevice" {
            if LEDGER.with(|l| l.borrow().empty_inventory) {
                r#"{"status":0,"body":{"devices":[]}}"#.to_string()
            } else {
                let (id, extra) = LEDGER.with(|l| {
                    let l = l.borrow();
                    (
                        if l.wrong_mat {
                            "other-mat"
                        } else {
                            "offline-mat"
                        },
                        l.multiple_mats,
                    )
                });
                format!(
                    r#"{{"status":0,"body":{{"devices":[{{"deviceid":"{id}","type":"Sleep Monitor","model":"Synthetic","model_id":1}}{}]}}}}"#,
                    if extra {
                        r#",{"deviceid":"second-mat","type":"Sleep Monitor","model":"Synthetic Two","model_id":2}"#
                    } else {
                        ""
                    }
                )
            }
        } else {
            panic!("Unexpected synthetic HTTP operation")
        };
        Ok(Response {
            status: 200,
            headers: vec![],
            body: if LEDGER.with(|l| l.borrow().second_account) {
                response
                    .replace("\"userid\":123", "\"userid\":456")
                    .into_bytes()
            } else {
                response.into_bytes()
            },
        })
    }
}
#[test]
fn automatic_component_two_mats_no_selection_and_isolated_events() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().multiple_mats = true);
    connect().unwrap();
    automatic::FLEET.with(|f| assert_eq!(f.borrow().sinks.len(), 2));
    LEDGER.with(|l| assert!(l.borrow().actions.is_empty()));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    let first = LEDGER.with(|l| l.borrow().actions[0].0.clone());
    let mut second = event(50);
    second.body = String::from_utf8(second.body)
        .unwrap()
        .replace("offline-mat", "second-mat")
        .into_bytes();
    assert_eq!(Plugin::on_webhook(second).status, 200);
    LEDGER.with(|l| {
        let l = l.borrow();
        assert_eq!(l.actions.len(), 2);
        assert_ne!(first, l.actions[1].0);
    });
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 2));
}

fn reset() {
    automatic::FLEET.with(|f| *f.borrow_mut() = automatic::Fleet::default());
    OWNER.with(|o| *o.borrow_mut() = Owner::new());

    LEDGER.with(|l| {
        *l.borrow_mut() = Ledger {
            time: 1000,
            ..Default::default()
        }
    });
}
fn event(appli: u8) -> InboundRequest {
    InboundRequest {
        method: "POST".into(),
        query: vec![],
        headers: vec![(
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: format!(
            "userid=123&deviceid=offline-mat&appli={appli}&date={}",
            clock().unwrap()
        )
        .into_bytes(),
    }
}
fn connect() -> Result<String, String> {
    Plugin::on_callback(
        "https://example.org/callback".into(),
        vec![("code".into(), "synthetic-code".into())],
        None,
    )
}
#[test]
fn alpha7_selection_migrates_alias_and_scoped_history_without_replay() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    let mut legacy = Owner::<HostStore, DeviceSink>::new();
    legacy.restore(&mut HostStore, 123, now().unwrap()).unwrap();
    reconcile_owner(&mut legacy).unwrap();
    legacy
        .discover(&mut HostTransport, &mut HostStore, now().unwrap())
        .unwrap();
    automatic::register("bedroom/withings_sleep").unwrap();
    let sink = DeviceSink::default();
    sink.set_target("bedroom/withings_sleep".into());
    legacy
        .accept(
            &mut HostStore,
            "offline-mat",
            legacy.generation(),
            HostStore,
            sink,
        )
        .unwrap();
    let e = event(50);
    let request = WebhookRequest {
        method: e.method,
        content_type: "application/x-www-form-urlencoded".into(),
        body: e.body,
    };
    assert_eq!(
        legacy.webhook(&request, now().unwrap(), 86400, 300).status,
        200
    );
    let selection = LEDGER.with(|l| l.borrow().storage["selection"].clone());
    Plugin::stop();
    LEDGER.with(|l| l.borrow_mut().multiple_mats = true);
    Plugin::init().unwrap();
    assert_eq!(id("offline-mat"), "bedroom/withings_sleep");
    assert_ne!(id("second-mat"), "bedroom/withings_sleep");
    assert!(panel().contains("Occupancy unknown"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 1));
    LEDGER.with(|l| l.borrow_mut().second_account = true);
    connect().unwrap();
    assert_ne!(id("offline-mat"), "bedroom/withings_sleep");
    assert_eq!(
        Plugin::on_webhook(other_event(456, "offline-mat", 50)).status,
        200
    );
    LEDGER.with(|l| {
        let l = l.borrow();
        assert_eq!(l.actions.len(), 2);
        assert_eq!(l.storage["selection"], selection);
    });
}

#[test]
fn registration_and_host_inventory_write_failures_are_retryable_without_actions() {
    for storage_failure in [false, true] {
        reset();
        Plugin::init().unwrap();
        LEDGER.with(|l| {
            let mut l = l.borrow_mut();
            l.fail_host_write = storage_failure;
            l.fail_registration = !storage_failure;
        });
        assert!(connect().is_err());
        assert_eq!(Plugin::on_webhook(event(50)).status, 503);
        LEDGER.with(|l| {
            let mut l = l.borrow_mut();
            assert!(l.actions.is_empty());
            l.fail_host_write = false;
            l.fail_registration = false;
        });
        retry().unwrap();
        assert_eq!(Plugin::on_webhook(event(50)).status, 200);
        LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 1));
    }
}
#[test]
fn removed_mat_on_cold_restart_and_failed_retirement_are_fail_closed() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().multiple_mats = true);
    connect().unwrap();
    assert_eq!(
        Plugin::on_webhook(other_event(123, "second-mat", 50)).status,
        200
    );
    let second = id("second-mat");
    LEDGER.with(|l| {
        let mut l = l.borrow_mut();
        l.multiple_mats = false;
        l.fail_availability = true;
    });
    assert!(retry().is_err());
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    LEDGER.with(|l| l.borrow_mut().fail_availability = false);
    retry().unwrap();
    LEDGER.with(|l| assert_eq!(l.borrow().available.get(&second), Some(&false)));
    // Simulate losing process memory without calling stop; persisted host IDs must revoke old devices.
    OWNER.with(|o| *o.borrow_mut() = Owner::new());
    automatic::FLEET.with(|f| *f.borrow_mut() = automatic::Fleet::default());
    LEDGER.with(|l| {
        let mut l = l.borrow_mut();
        l.available.insert(second.clone(), true);
        l.secrets.insert("account".into(), "{".into());
    });
    Plugin::init().unwrap();
    LEDGER.with(|l| assert!(l.borrow().available.values().all(|v| !*v)));
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}

#[test]
fn changed_invalid_account_record_revokes_running_owner_on_timer() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| {
        l.borrow_mut().secrets.insert("account".into(), "{".into());
    });
    Plugin::on_timer("lifecycle".into());
    assert_eq!(Plugin::on_webhook(event(51)).status, 503);
    LEDGER.with(|l| assert!(l.borrow().available.values().all(|v| !*v)));
}
fn context() -> RenderContext {
    RenderContext {
        room: None,
        person: None,
    }
}
fn tree() -> UiTree {
    Plugin::render("withings".into(), context()).unwrap()
}
fn panel() -> String {
    let t = tree();
    assert_eq!(t.nodes[0].children.len(), t.nodes.len() - 1);
    t.nodes
        .iter()
        .filter_map(|n| match &n.kind {
            crate::niles::integration::ui::NodeKind::Text(t) => Some(t.value.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn retry_id() -> String {
    tree()
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            crate::niles::integration::ui::NodeKind::Button(b) if b.id.starts_with("retry:") => {
                Some(b.id.clone())
            }
            _ => None,
        })
        .unwrap()
}
fn retry() -> Result<ActionResult, String> {
    Plugin::on_action(
        "withings".into(),
        retry_id(),
        ActionValue::Pressed,
        context(),
    )
}
fn id(device: &str) -> String {
    automatic::FLEET.with(|f| f.borrow().sinks[device].target().unwrap())
}
fn other_event(account: u64, device: &str, appli: u8) -> InboundRequest {
    let mut e = event(appli);
    e.body = String::from_utf8(e.body)
        .unwrap()
        .replace("userid=123", &format!("userid={account}"))
        .replace("offline-mat", device)
        .into_bytes();
    e
}
#[test]
fn actual_init_without_credentials_keeps_connect_and_blocks_events() {
    reset();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    assert!(panel().contains("Connect Withings"));
    LEDGER.with(|l| {
        let l = l.borrow();
        assert!(l.registered.is_empty());
        assert!(l.requests.is_empty());
        assert_eq!(l.timers, ["lifecycle"]);
    });
}
#[test]
fn actual_callback_webhook_restart_and_denial() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    let device = id("offline-mat");
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| {
        let l = l.borrow();
        assert_eq!(l.actions, [(device.clone(), "bed_in".into())]);
        assert!(l.readings.contains(&("in_bed".into(), 1.0)));
        assert_eq!(l.available.get(&device), Some(&true));
    });
    Plugin::stop();
    Plugin::init().unwrap();
    assert!(panel().contains("Occupancy unknown"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 1));
    Plugin::on_callback(
        "https://example.org/callback".into(),
        vec![("error".into(), "access_denied".into())],
        None,
    )
    .unwrap();
    assert_eq!(Plugin::on_webhook(event(51)).status, 503);
    Plugin::on_timer("lifecycle".into());
    assert_eq!(Plugin::on_webhook(event(51)).status, 503);
    LEDGER.with(|l| assert_eq!(l.borrow().available.get(&device), Some(&false)));
}
#[test]
fn account_onboarding_automatic_without_manual_settings() {
    reset();
    Plugin::init().unwrap();
    assert!(connect().unwrap().contains("automatically"));
    assert_eq!(automatic::FLEET.with(|f| f.borrow().sinks.len()), 1);
    assert!(
        tree()
            .nodes
            .iter()
            .all(|n| !matches!(n.kind, crate::niles::integration::ui::NodeKind::Choice(_)))
    );
    LEDGER.with(|l| assert!(l.borrow().actions.is_empty()));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    let original = id("offline-mat");
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(id("offline-mat"), original);
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 1));
}
#[test]
fn corrupt_account_and_corrupt_legacy_selection_fail_closed() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    LEDGER.with(|l| {
        l.borrow_mut()
            .storage
            .insert("selection".into(), "{".into());
    });
    assert!(retry().is_err());
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    LEDGER.with(|l| {
        l.borrow_mut().storage.remove("selection");
    });
    retry().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| {
        l.borrow_mut().secrets.insert("account".into(), "{".into());
    });
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}
#[test]
fn timer_rotation_final_save_failure_revokes_and_restart_refuses() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().multiple_mats = true);
    connect().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| {
        let mut l = l.borrow_mut();
        l.time = 4601;
        l.fail_final = true;
    });
    Plugin::on_timer("lifecycle".into());
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    LEDGER.with(|l| {
        let l = l.borrow();
        assert!(l.secrets["account"].contains("\"refresh_pending\":true"));
        assert_eq!(
            l.requests
                .iter()
                .filter(|r| r.contains("grant_type=refresh_token"))
                .count(),
            1
        );
        assert!(l.available.values().all(|v| !*v));
    });
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}
#[test]
fn rendered_retry_retains_fresh_state_and_stale_control_is_rejected() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    assert!(retry().unwrap().rerender);
    assert!(panel().contains("In bed"));
    let stale = retry_id();
    connect().unwrap();
    assert!(Plugin::on_action("withings".into(), stale, ActionValue::Pressed, context()).is_err());
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 1));
}
#[test]
fn retry_distinguishes_empty_and_failed_discovery_and_recovers() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().empty_inventory = true);
    connect().unwrap();
    assert!(panel().contains("0 supported sleep mats"));
    LEDGER.with(|l| {
        let mut l = l.borrow_mut();
        l.empty_inventory = false;
        l.discovery_error = true;
    });
    assert!(retry().is_err());
    assert!(panel().contains("Device discovery failed"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    LEDGER.with(|l| l.borrow_mut().discovery_error = false);
    retry().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
}
#[test]
fn multiple_mats_second_mat_entry_duplicate_exit_and_removal() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().multiple_mats = true);
    connect().unwrap();
    let second = id("second-mat");
    assert_eq!(
        Plugin::on_webhook(other_event(123, "second-mat", 50)).status,
        200
    );
    assert_eq!(
        Plugin::on_webhook(other_event(123, "second-mat", 50)).status,
        200
    );
    LEDGER.with(|l| l.borrow_mut().time += 1);
    assert_eq!(
        Plugin::on_webhook(other_event(123, "second-mat", 51)).status,
        200
    );
    LEDGER.with(|l| {
        assert_eq!(
            l.borrow().actions,
            [
                (second.clone(), "bed_in".into()),
                (second.clone(), "bed_out".into())
            ]
        );
        l.borrow_mut().multiple_mats = false;
    });
    retry().unwrap();
    assert_ne!(
        Plugin::on_webhook(other_event(123, "second-mat", 50)).status,
        200
    );
    LEDGER.with(|l| assert_eq!(l.borrow().available.get(&second), Some(&false)));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
}
#[test]
fn account_restrictions_and_damaged_records_fail_closed() {
    for expected in ["bad", "999"] {
        reset();
        Plugin::init().unwrap();
        LEDGER.with(|l| l.borrow_mut().configured_user = Some(expected.into()));
        assert!(connect().is_err());
        Plugin::on_timer("lifecycle".into());
        assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    }
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    let valid = LEDGER.with(|l| l.borrow().secrets["account"].clone());
    for damaged in [
        "{".to_string(),
        valid.replace("\"version\":1", "\"version\":99"),
    ] {
        Plugin::stop();
        LEDGER.with(|l| {
            l.borrow_mut().secrets.insert("account".into(), damaged);
        });
        let before = LEDGER.with(|l| l.borrow().requests.len());
        Plugin::init().unwrap();
        assert_eq!(Plugin::on_webhook(event(50)).status, 503);
        assert_eq!(LEDGER.with(|l| l.borrow().requests.len()), before);
    }
}
#[test]
fn replacement_account_has_independent_identity_history_and_restart() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    let old = id("offline-mat");
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| l.borrow_mut().second_account = true);
    connect().unwrap();
    let new = id("offline-mat");
    assert_ne!(old, new);
    assert_ne!(Plugin::on_webhook(event(50)).status, 200);
    assert_eq!(
        Plugin::on_webhook(other_event(456, "offline-mat", 50)).status,
        200
    );
    LEDGER.with(|l| {
        let l = l.borrow();
        assert_eq!(l.actions.len(), 2);
        assert_eq!(l.available.get(&old), Some(&false));
    });
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(id("offline-mat"), new);
    assert_ne!(Plugin::on_webhook(event(50)).status, 200);
    assert_eq!(
        Plugin::on_webhook(other_event(456, "offline-mat", 50)).status,
        200
    );
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 2));
}
#[test]
fn subscription_failure_recovers_on_timer_without_reconnecting() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().subscription_error = true);
    assert!(connect().is_err());
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    assert!(!panel().contains("subscriptions ready"));
    LEDGER.with(|l| l.borrow_mut().subscription_error = false);
    Plugin::on_timer("lifecycle".into());
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    assert!(panel().contains("subscriptions ready"));
}
#[test]
fn panel_tracks_fresh_events_and_recovery_without_exposing_identifiers() {
    reset();
    Plugin::init().unwrap();
    assert!(panel().contains("Connect Withings"));
    connect().unwrap();
    assert!(panel().contains("Occupancy unknown"));
    assert!(panel().contains("subscriptions ready"));
    assert!(!panel().contains("offline-mat"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    assert!(panel().contains("In bed"));
    Plugin::stop();
    Plugin::init().unwrap();
    assert!(panel().contains("Occupancy unknown"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    assert!(panel().contains("Occupancy unknown"));
    LEDGER.with(|l| l.borrow_mut().time += 1);
    assert_eq!(Plugin::on_webhook(event(51)).status, 200);
    assert!(panel().contains("Out of bed"));
}
