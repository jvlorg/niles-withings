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
                "room" => "bedroom",
                "user_id" => "123",
                "device_id" => "offline-mat",
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
        LEDGER.with(|l| {
            l.borrow_mut().storage.insert(k.into(), v.into());
        });
        Ok(())
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
        LEDGER.with(|l| l.borrow_mut().requests.push(body.into()));
        let response = if body.contains("grant_type=") {
            r#"{"status":0,"body":{"userid":123,"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600,"client_id":"fixture-client"}}"#.to_string()
        } else if body.starts_with("action=list") {
            assert!(
                r.headers
                    .iter()
                    .any(|(k, v)| k == "authorization" && v == "Bearer fixture-access")
            );
            r#"{"status":0,"body":{"profiles":[{"appli":50,"callbackurl":"https://example.org/hook","comment":"Niles Withings"},{"appli":51,"callbackurl":"https://example.org/hook","comment":"Niles Withings"}]}}"#.to_string()
        } else if body == "action=getdevice" {
            let id = LEDGER.with(|l| {
                if l.borrow().wrong_mat {
                    "other-mat"
                } else {
                    "offline-mat"
                }
            });
            format!(
                r#"{{"status":0,"body":{{"devices":[{{"deviceid":"{id}","type":"Sleep Monitor","model":"Synthetic","model_id":1}}]}}}}"#
            )
        } else {
            panic!("Unexpected synthetic HTTP operation")
        };
        Ok(Response {
            status: 200,
            headers: vec![],
            body: response.into_bytes(),
        })
    }
}
fn reset() {
    OWNER.with(|o| *o.borrow_mut() = Owner::new());
    SINK.with(|s| *s.id.borrow_mut() = None);
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
fn absent_configured_mat_is_not_success() {
    reset();
    Plugin::init().unwrap();
    LEDGER.with(|l| l.borrow_mut().wrong_mat = true);
    assert!(connect().is_err());
    assert_eq!(Plugin::on_webhook(event(50)).status, 503);
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
    assert!(panel().contains("Reconnect Withings"));
    assert!(panel().contains("Occupancy unknown"));
    connect().unwrap();
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
