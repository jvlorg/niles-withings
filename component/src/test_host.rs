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
    fail_selection_write: bool,
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
            if l.fail_selection_write && k == "selection" {
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
        LEDGER.with(|l| l.borrow_mut().registered.push(d.id.clone()));
        Ok(())
    }
    pub fn set_available(id: &str, value: bool) -> Result<(), ()> {
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
fn reset() {
    OWNER.with(|o| *o.borrow_mut() = Owner::new());
    SINK.with(|s| *s.id.borrow_mut() = None);
    SINK.with(|s| *s.occupancy.borrow_mut() = None);
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
fn choose_rendered(index: usize) -> String {
    let tree = Plugin::render(
        "withings".into(),
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    let (control, device) = tree
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            crate::niles::integration::ui::NodeKind::Choice(c) => {
                Some((c.id.clone(), c.options[index].value.clone()))
            }
            _ => None,
        })
        .unwrap();
    Plugin::on_action(
        "withings".into(),
        control,
        ActionValue::Chosen(device.clone()),
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    device
}
#[test]
fn actual_init_without_credentials_keeps_connect_and_blocks_events() {
    reset();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    LEDGER.with(|l| {
        let l = l.borrow();
        assert_eq!(l.registered, ["bedroom/withings_sleep"]);
        assert_eq!(l.available.get("bedroom/withings_sleep"), Some(&false));
        assert!(l.requests.is_empty());
        assert_eq!(l.timers, ["lifecycle"]);
    });
}
#[test]
fn actual_callback_webhook_restart_and_denial() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    choose_rendered(0);
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| {
        let l = l.borrow();
        assert_eq!(
            l.actions,
            [("bedroom/withings_sleep".into(), "bed_in".into())]
        );
        assert!(l.readings.contains(&("in_bed".into(), 1.0)));
        assert_eq!(l.available.get("bedroom/withings_sleep"), Some(&true));
    });
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 1));
    Plugin::on_callback(
        "https://example.org/callback".into(),
        vec![("error".into(), "access_denied".into())],
        None,
    )
    .unwrap();
    assert_eq!(Plugin::on_webhook(event(51)).status, 503);
    LEDGER.with(|l| {
        assert_eq!(
            l.borrow().available.get("bedroom/withings_sleep"),
            Some(&false)
        )
    });
}
#[test]
fn account_onboarding_without_user_setting_selects_and_restores_mat() {
    reset();
    Plugin::init().unwrap();
    assert!(connect().unwrap().contains("select a mat"));
    let selected = choose_rendered(0);
    assert_eq!(selected, "offline-mat");
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    Plugin::stop();
    Plugin::init().unwrap();
    assert!(panel().contains("Configured sleep mat selected"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 1));
}
#[test]
fn malformed_pending_and_account_switch_do_not_reuse_saved_mat_or_deliver() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    choose_rendered(0);
    LEDGER.with(|l| l.borrow_mut().secrets.insert("account".into(), "{".into()));
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}
#[test]
fn timer_rotation_final_save_failure_revokes_and_restart_refuses() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
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
    });
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}
#[test]
fn interactive_choice_uses_actual_rendered_control_and_persists_then_delivers() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    let tree = Plugin::render(
        "withings".into(),
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    let (control, candidate) = tree
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            crate::niles::integration::ui::NodeKind::Choice(c) => {
                Some((c.id.clone(), c.options[0].value.clone()))
            }
            _ => None,
        })
        .expect("render includes candidate selection");
    assert!(
        Plugin::on_action(
            "withings".into(),
            control,
            ActionValue::Chosen(candidate),
            RenderContext {
                room: None,
                person: None
            }
        )
        .unwrap()
        .rerender
    );
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
}
#[test]
fn stale_rendered_choice_and_failed_storage_never_activate() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    let tree = Plugin::render(
        "withings".into(),
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    let (control, candidate) = tree
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            crate::niles::integration::ui::NodeKind::Choice(c) => {
                Some((c.id.clone(), c.options[0].value.clone()))
            }
            _ => None,
        })
        .unwrap();
    let generation = control.rsplit(':').next().unwrap();
    Plugin::on_action(
        "withings".into(),
        format!("switch:{generation}"),
        ActionValue::Pressed,
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    assert!(
        Plugin::on_action(
            "withings".into(),
            control.clone(),
            ActionValue::Chosen(candidate.clone()),
            RenderContext {
                room: None,
                person: None
            }
        )
        .is_err()
    );
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    LEDGER.with(|l| l.borrow_mut().fail_selection_write = true);
    let tree = Plugin::render(
        "withings".into(),
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    let (control, candidate) = tree
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            crate::niles::integration::ui::NodeKind::Choice(c) => {
                Some((c.id.clone(), c.options[0].value.clone()))
            }
            _ => None,
        })
        .unwrap();
    assert!(
        Plugin::on_action(
            "withings".into(),
            control,
            ActionValue::Chosen(candidate),
            RenderContext {
                room: None,
                person: None
            }
        )
        .is_err()
    );
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}
#[test]
fn retry_distinguishes_empty_and_failed_discovery() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().empty_inventory = true);
    connect().unwrap();
    let tree = Plugin::render(
        "withings".into(),
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    assert!(tree.nodes.iter().any(|n| matches!(&n.kind, crate::niles::integration::ui::NodeKind::Text(t) if t.value.contains("No supported"))));
    let retry = tree
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            crate::niles::integration::ui::NodeKind::Button(b) if b.id.starts_with("retry:") => {
                Some(b.id.clone())
            }
            _ => None,
        })
        .unwrap();
    LEDGER.with(|l| {
        let mut l = l.borrow_mut();
        l.empty_inventory = false;
        l.discovery_error = true;
    });
    Plugin::on_action(
        "withings".into(),
        retry,
        ActionValue::Pressed,
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap_err();
    assert!(panel().contains("discovery is unavailable"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}
#[test]
fn multiple_candidates_are_rendered_and_selected_explicitly() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().multiple_mats = true);
    connect().unwrap();
    let tree = Plugin::render(
        "withings".into(),
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    let choice = tree
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            crate::niles::integration::ui::NodeKind::Choice(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(choice.options.len(), 2);
    let selected = choose_rendered(1);
    let mut bed_in = event(50);
    bed_in.body = String::from_utf8(bed_in.body)
        .unwrap()
        .replace("offline-mat", &selected)
        .into_bytes();
    assert_eq!(Plugin::on_webhook(bed_in.clone()).status, 200);
    assert_eq!(Plugin::on_webhook(bed_in).status, 200);
    LEDGER.with(|l| assert_eq!(l.borrow().actions.len(), 1));
    LEDGER.with(|l| l.borrow_mut().time += 1);
    let mut bed_out = event(51);
    bed_out.body = String::from_utf8(bed_out.body)
        .unwrap()
        .replace("offline-mat", &selected)
        .into_bytes();
    assert_eq!(Plugin::on_webhook(bed_out).status, 200);
    LEDGER.with(|l| {
        assert_eq!(
            l.borrow().actions,
            [
                ("bedroom/withings_sleep".into(), "bed_in".into()),
                ("bedroom/withings_sleep".into(), "bed_out".into()),
            ]
        )
    });
}
#[test]
fn legacy_configured_mat_activates_once_and_callback_is_truthful() {
    reset();
    LEDGER.with(|l| l.borrow_mut().legacy_id = true);
    Plugin::init().unwrap();
    assert!(connect().unwrap().contains("sleep mat activated"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    reset();
    Plugin::init().unwrap();
    assert!(connect().unwrap().contains("select a mat"));
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}
#[test]
fn account_restrictions_and_damaged_records_fail_closed() {
    for expected in ["bad", "999"] {
        reset();
        Plugin::init().unwrap();
        LEDGER.with(|l| l.borrow_mut().configured_user = Some(expected.into()));
        assert!(connect().is_err());
        assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    }
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    choose_rendered(0);
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
        assert!(panel().contains("Reconnect Withings"));
    }
}
#[test]
fn replacement_account_does_not_inherit_mat_or_old_events() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    choose_rendered(0);
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    let saved = LEDGER.with(|l| l.borrow().storage["selection"].clone());
    LEDGER.with(|l| l.borrow_mut().second_account = true);
    let _ = connect(); // Foreign persisted selection may explicitly block setup.
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    assert!(OWNER.with(|o| o.borrow().presentation().selected.is_none()));
    assert_eq!(
        LEDGER.with(|l| l.borrow().storage["selection"].clone()),
        saved
    );
    Plugin::stop();
    Plugin::init().unwrap();
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
}
#[test]
fn subscription_failure_recovers_on_timer_without_reconnecting() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().subscription_error = true);
    assert!(connect().is_err());
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
    LEDGER.with(|l| l.borrow_mut().subscription_error = false);
    Plugin::on_timer("lifecycle".into());
    choose_rendered(0);
    assert_eq!(Plugin::on_webhook(event(50)).status, 200);
    assert!(panel().contains("subscriptions ready"));
}
#[test]
fn replacement_account_explicit_retry_selects_without_old_event_delivery() {
    reset();
    Plugin::init().unwrap();
    connect().unwrap();
    choose_rendered(0);
    LEDGER.with(|l| l.borrow_mut().second_account = true);
    let _ = connect();
    // The new account offers its own rendered choice, never the saved binding.
    assert!(OWNER.with(|o| o.borrow().presentation().selected.is_none()));
    choose_rendered(0);
    assert_ne!(Plugin::on_webhook(event(50)).status, 200);
    let mut current = event(50);
    current.body = String::from_utf8(current.body)
        .unwrap()
        .replace("userid=123", "userid=456")
        .into_bytes();
    assert_eq!(Plugin::on_webhook(current).status, 200);
    Plugin::stop();
    Plugin::init().unwrap();
    assert_ne!(Plugin::on_webhook(event(50)).status, 200);
}
fn panel() -> String {
    let tree = Plugin::render(
        "withings".into(),
        RenderContext {
            room: None,
            person: None,
        },
    )
    .unwrap();
    assert_eq!(tree.nodes[0].children.len(), tree.nodes.len() - 1);
    tree.nodes
        .iter()
        .filter_map(|n| match &n.kind {
            crate::niles::integration::ui::NodeKind::Text(t) => Some(t.value.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn panel_tracks_fresh_events_and_recovery_without_exposing_identifiers() {
    reset();
    Plugin::init().unwrap();
    assert!(panel().contains("Connect Withings"));
    assert!(panel().contains("Occupancy unknown"));
    connect().unwrap();
    choose_rendered(0);
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
