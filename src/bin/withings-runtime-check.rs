use niles_withings::{
    application::{CallbackResult, WebhookResponse},
    lifecycle::{FixedClock, Status},
    runtime::Owner,
    withings_client::{CallbackUrl, HttpResponse, Secret},
};
mod runtime_fixture {
    include!("../../tests/support/runtime_fixture.rs");
}
use runtime_fixture::{Shared, SyntheticTransport, device_reply, event};

const NOW: i64 = 1_000;

fn run(break_refresh: bool) -> Result<(), &'static str> {
    let shared = Shared::default();
    let mut owner = Owner::<Shared, Shared>::new();
    let callback =
        CallbackUrl::parse("https://example.org/callback").map_err(|_| "callback URL")?;
    let mut transport = SyntheticTransport { requests: 0, replies: [
        HttpResponse { status: 200, body: br#"{"status":0,"body":{"userid":123,"access_token":"fixture-only","refresh_token":"fixture-only-refresh","expires_in":3600,"client_id":"fixture-client"}}"#.to_vec() },
        device_reply(),
    ].into() };
    let secret = Secret::new("fixture-only-secret").map_err(|_| "secret")?;
    let mut shared_store = shared.clone();
    let result = owner
        .callback(
            &mut transport,
            &mut shared_store,
            &callback,
            &[("code".into(), "fixture-only-code".into())],
            None,
            "fixture-client",
            &secret,
            NOW,
        )
        .map_err(|_| "callback")?;
    if result != (CallbackResult::Stored { user_id: 123 }) {
        return Err("callback result");
    }
    if shared.0.borrow().accounts.is_none() {
        return Err("callback readback");
    }
    println!("PASS callback → store readback");

    let mut selections = shared.clone();
    owner
        .discover(&mut transport, &mut selections, NOW)
        .map_err(|_| "discovery")?;
    println!("PASS discover");
    let generation = owner.generation();
    owner
        .accept(
            &mut selections,
            "offline-mat",
            generation,
            shared.clone(),
            shared.clone(),
        )
        .map_err(|_| "accept")?;
    if shared.0.borrow().selections.is_none() {
        return Err("selection readback");
    }
    println!("PASS accept");
    let profile = || {
        HttpResponse { status: 200, body: br#"{"status":0,"body":{"profiles":[{"appli":50,"callbackurl":"https://example.org/callback","comment":"offline"},{"appli":51,"callbackurl":"https://example.org/callback","comment":"offline"}]}}"#.to_vec() }
    };
    let mut subscriptions = SyntheticTransport {
        requests: 0,
        replies: [profile(), profile(), profile(), profile()].into(),
    };
    let status = owner.reconcile(
        &mut subscriptions,
        &mut shared_store,
        &FixedClock(NOW),
        "fixture-client",
        &secret,
        &callback,
        "offline",
    );
    if status.status != Status::Ready || subscriptions.requests != 4 {
        return Err("subscriptions readback");
    }
    println!("PASS subscriptions readback");

    let request = event(NOW);
    if owner.webhook(&request, NOW, 0, 0).status != 200 {
        return Err("event delivery");
    }
    if shared.0.borrow().actions != ["bed_in"] {
        return Err("event action");
    }
    println!("PASS event");
    drop(owner);

    let mut owner = Owner::<Shared, Shared>::new();
    let mut account_store = shared.clone();
    owner
        .restore(&mut account_store, 123, NOW)
        .map_err(|_| "restore")?;
    println!("PASS drop owner → restore (process restart simulated; shared in-memory persistence)");
    let mut discovery = SyntheticTransport {
        replies: [device_reply()].into(),
        requests: 0,
    };
    owner
        .discover(&mut discovery, &mut selections, NOW)
        .map_err(|_| "restart discovery")?;
    owner
        .activate(&mut selections, 123, shared.clone(), shared.clone())
        .map_err(|_| "activate")?;
    println!("PASS activate");
    if owner.webhook(&request, NOW, 0, 0).status != 200 || shared.0.borrow().actions != ["bed_in"] {
        return Err("duplicate was not silent");
    }
    println!("PASS duplicate silent");
    let mut exit = event(NOW + 1);
    exit.body = format!("userid=123&deviceid=offline-mat&appli=51&date={}", NOW + 1).into_bytes();
    if owner.webhook(&exit, NOW + 1, 0, 0).status != 200
        || shared.0.borrow().actions != ["bed_in", "bed_out"]
    {
        return Err("exit action");
    }
    println!("PASS exit event");

    let url = CallbackUrl::parse("https://example.org/callback").map_err(|_| "callback URL")?;
    let client_secret = Secret::new("fixture-only-secret").map_err(|_| "secret")?;
    let mut refresh = SyntheticTransport {
        replies: [HttpResponse { status: 200, body: if break_refresh { b"invalid".to_vec() } else { br#"{"status":0,"body":{"userid":123,"access_token":"rotated-fixture","refresh_token":"rotated-fixture-refresh","expires_in":3600}}"#.to_vec() }}].into(),
        requests: 0,
    };
    let mut secret_store = shared.clone();
    shared.0.borrow_mut().fail_final_save = true;
    let status = owner.reconcile(
        &mut refresh,
        &mut secret_store,
        &FixedClock(NOW + 3500),
        "fixture-client",
        &client_secret,
        &url,
        "offline",
    );
    if status.status != Status::RefreshPersistenceFailed || refresh.requests != 1 {
        return Err("refresh did not fail closed");
    }
    let response: WebhookResponse = owner.webhook(&event(NOW + 1), NOW + 1, 0, 0);
    if response.status != 503 {
        return Err("delivery not blocked");
    }
    let mut restarted = Owner::<Shared, Shared>::new();
    if restarted
        .restore(&mut secret_store, 123, NOW + 3500)
        .is_ok()
    {
        return Err("refresh pending restart accepted");
    }
    println!("PASS refresh final-save failure → delivery blocked");
    Ok(())
}

fn main() {
    let deliberate_failure = std::env::args().any(|arg| arg == "--break-refresh-response");
    println!("OFFLINE SYNTHETIC NOT HOST0.3");
    if let Err(stage) = run(deliberate_failure) {
        eprintln!("FAIL {stage}");
        std::process::exit(1);
    }
}
