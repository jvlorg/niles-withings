#[allow(dead_code)]
mod fixture {
    include!("support/runtime_fixture.rs");
}
use fixture::{Shared, SyntheticTransport};
use niles_withings::{
    application::WebhookRequest,
    lifecycle::{FixedClock, Status},
    runtime::Owner,
    withings_client::{CallbackUrl, HttpResponse, Secret},
};
fn reply(v: serde_json::Value) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: serde_json::to_vec(&v).unwrap(),
    }
}
fn inventory() -> HttpResponse {
    reply(
        serde_json::json!({"status":0,"body":{"devices":[{"deviceid":"a","type":"Sleep Monitor","model":"Synthetic","model_id":63},{"deviceid":"b","type":"Sleep Monitor","model":"Synthetic","model_id":63}]}}),
    )
}
fn event(id: &str) -> WebhookRequest {
    WebhookRequest {
        method: "POST".into(),
        content_type: "application/x-www-form-urlencoded".into(),
        body: format!("userid=123&deviceid={id}&appli=50&date=1000").into_bytes(),
    }
}
fn ready(o: &mut Owner<Shared, Shared>, s: &mut Shared) {
    let profile = || {
        reply(
            serde_json::json!({"status":0,"body":{"profiles":[{"appli":50,"callbackurl":"https://example.org/callback","comment":"offline"},{"appli":51,"callbackurl":"https://example.org/callback","comment":"offline"}]}}),
        )
    };
    let mut t = SyntheticTransport {
        requests: 0,
        replies: [profile(), profile(), profile(), profile()].into(),
    };
    assert_eq!(
        o.reconcile(
            &mut t,
            s,
            &FixedClock(1000),
            "fixture-client",
            &Secret::new("fixture-secret").unwrap(),
            &CallbackUrl::parse("https://example.org/callback").unwrap(),
            "offline"
        )
        .status,
        Status::Ready
    );
}
#[test]
fn automatic_callback_two_devices_restart_expiry_and_denial() {
    let mut store = Shared::default();
    let a = Shared::default();
    let b = Shared::default();
    let mut owner = Owner::new();
    let cb = CallbackUrl::parse("https://example.org/callback").unwrap();
    let secret = Secret::new("fixture-secret").unwrap();
    let mut tokens=SyntheticTransport{requests:0,replies:[reply(serde_json::json!({"status":0,"body":{"userid":123,"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600}}))].into()};
    owner
        .callback(
            &mut tokens,
            &mut store,
            &cb,
            &[("code".into(), "fixture-code".into())],
            None,
            "fixture-client",
            &secret,
            1000,
        )
        .unwrap();
    let discover = |o: &mut Owner<Shared, Shared>| {
        let mut t = SyntheticTransport {
            requests: 0,
            replies: [inventory()].into(),
        };
        o.discover_automatic(
            &mut t,
            1000,
            |_| store.clone(),
            |d| {
                if d.device_id == "a" {
                    a.clone()
                } else {
                    b.clone()
                }
            },
        )
        .unwrap();
    };
    discover(&mut owner);
    assert_eq!(owner.webhook(&event("a"), 1000, 10, 0).status, 503);
    ready(&mut owner, &mut store.clone());
    assert_eq!(owner.webhook(&event("a"), 1000, 10, 0).status, 200);
    assert_eq!(a.0.borrow().actions, vec!["bed_in"]);
    assert!(b.0.borrow().actions.is_empty());
    assert_eq!(owner.webhook(&event("b"), 1000, 10, 0).status, 200);
    assert_eq!(b.0.borrow().actions, vec!["bed_in"]);
    let mut restored = Owner::new();
    restored.restore(&mut store.clone(), 123, 1000).unwrap();
    discover(&mut restored);
    ready(&mut restored, &mut store.clone());
    assert_eq!(restored.webhook(&event("a"), 1000, 10, 0).status, 200);
    assert_eq!(a.0.borrow().actions.len(), 1);
    let mut no_io = SyntheticTransport::default();
    assert!(
        restored
            .discover_automatic(&mut no_io, 4600, |_| store.clone(), |_| a.clone())
            .is_err()
    );
    assert_eq!(no_io.requests, 0);
    assert_eq!(restored.webhook(&event("b"), 1000, 10, 0).status, 503);
    restored
        .callback(
            &mut no_io,
            &mut store.clone(),
            &cb,
            &[("error".into(), "access_denied".into())],
            None,
            "fixture-client",
            &secret,
            1000,
        )
        .unwrap();
    assert_eq!(restored.webhook(&event("a"), 1000, 10, 0).status, 503);
}
