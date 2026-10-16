use niles_withings::{
    adapter::{Reading, Sink, Store},
    application::{
        ApplicationError, SecureAccountRecord, SelectionStore, SetupPresentation, SetupResolution,
        accept_selection, adapter_from_binding, setup_discover,
    },
    lifecycle::TokenMetadata,
    withings_client::{HttpRequest, HttpResponse, HttpTransport, Secret, TokenSet},
};

#[derive(Default)]
struct Transport {
    replies: std::collections::VecDeque<HttpResponse>,
    calls: usize,
}
impl HttpTransport for Transport {
    type Error = ();
    fn post(&mut self, request: HttpRequest) -> Result<HttpResponse, ()> {
        self.calls += 1;
        assert_eq!(request.form, "action=getdevice");
        self.replies.pop_front().ok_or(())
    }
}
#[derive(Default)]
struct Selections {
    value: Option<String>,
    fail_save: bool,
}
impl SelectionStore for Selections {
    type Error = ();
    fn load_selection(&mut self) -> Result<Option<String>, ()> {
        Ok(self.value.clone())
    }
    fn save_selection(&mut self, value: &str) -> Result<(), ()> {
        if self.fail_save {
            return Err(());
        }
        self.value = Some(value.into());
        Ok(())
    }
}
#[derive(Default)]
struct StateStore {
    value: Option<String>,
}
impl Store for StateStore {
    type Error = &'static str;
    fn load(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(self.value.clone())
    }
    fn save(&mut self, state: &str) -> Result<(), Self::Error> {
        self.value = Some(state.into());
        Ok(())
    }
}
impl niles_withings::coordinator::ScopedBackend for StateStore {
    type Error = &'static str;
    fn load_value(&mut self, _: &str) -> Result<Option<String>, Self::Error> {
        self.load()
    }
    fn save_value(&mut self, _: &str, value: &str) -> Result<(), Self::Error> {
        self.save(value)
    }
}
#[derive(Default)]
struct Events {
    actions: Vec<&'static str>,
}
impl Sink for Events {
    type Error = &'static str;
    fn publish_reading(&mut self, _: Reading) -> Result<(), &'static str> {
        Ok(())
    }
    fn publish_action(&mut self, action: &'static str) -> Result<(), &'static str> {
        self.actions.push(action);
        Ok(())
    }
}
fn record(user: u64) -> SecureAccountRecord {
    SecureAccountRecord::from_tokens(
        &TokenSet {
            user_id: user,
            access_token: Secret::new("synthetic-access").unwrap(),
            refresh_token: Secret::new("synthetic-refresh").unwrap(),
            expires_in: 3600,
        },
        TokenMetadata::new_for_account(1000, 3600).unwrap(),
    )
    .unwrap()
}
fn devices(rows: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: format!(r#"{{"status":0,"body":{{"devices":[{rows}]}}}}"#).into_bytes(),
    }
}
#[derive(Clone, Default)]
struct Shared(std::rc::Rc<std::cell::RefCell<Ledger>>);
#[derive(Default)]
struct Ledger {
    values: std::collections::BTreeMap<String, String>,
    actions: Vec<&'static str>,
    readings: Vec<Reading>,
    fail_load: bool,
    fail_save: bool,
}
impl niles_withings::coordinator::ScopedBackend for Shared {
    type Error = &'static str;
    fn load_value(&mut self, k: &str) -> Result<Option<String>, Self::Error> {
        let d = self.0.borrow();
        if d.fail_load {
            Err("load")
        } else {
            Ok(d.values.get(k).cloned())
        }
    }
    fn save_value(&mut self, k: &str, v: &str) -> Result<(), Self::Error> {
        let mut d = self.0.borrow_mut();
        if d.fail_save {
            return Err("save");
        };
        d.values.insert(k.into(), v.into());
        Ok(())
    }
}
impl Sink for Shared {
    type Error = &'static str;
    fn publish_reading(&mut self, r: Reading) -> Result<(), Self::Error> {
        self.0.borrow_mut().readings.push(r);
        Ok(())
    }
    fn publish_action(&mut self, a: &'static str) -> Result<(), Self::Error> {
        self.0.borrow_mut().actions.push(a);
        Ok(())
    }
}
#[test]
fn coordinator_scopes_mat_switch_restart_and_failures() {
    use niles_withings::{
        application::WebhookRequest,
        coordinator::{Coordinator, ScopedStore, presence_namespace},
    };
    let shared = Shared::default();
    let mut selections = Selections::default();
    let mut owner = Coordinator::<ScopedStore<Shared>, Shared>::default();
    let rows = format!("{MAT},{}", MAT.replace("synthetic-mat", "matB"));
    let mut transport = Transport {
        replies: (0..8).map(|_| devices(&rows)).collect(),
        calls: 0,
    };
    let event = |user, mat, appli, date| WebhookRequest {
        method: "POST".into(),
        content_type: "application/x-www-form-urlencoded".into(),
        body: format!("userid={user}&deviceid={mat}&appli={appli}&date={date}").into_bytes(),
    };
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
        .unwrap();
    for mat in ["synthetic-mat", "matB", "synthetic-mat"] {
        owner.switch_device();
        assert_eq!(
            owner.webhook(&event(123, mat, 50, 1001), 1001, 0, 0).status,
            503
        );
        owner
            .discover(&mut transport, &mut selections, &record(123), 1001)
            .unwrap();
        owner
            .accept_scoped(
                &mut selections,
                mat,
                owner.generation(),
                shared.clone(),
                shared.clone(),
            )
            .unwrap();
        assert_eq!(
            owner.webhook(&event(123, mat, 50, 1001), 1001, 0, 0).status,
            200
        );
    }
    assert_eq!(shared.0.borrow().actions, vec!["bed_in", "bed_in"]);
    assert!(matches!(
        shared.0.borrow().readings.last(),
        Some(Reading::Unknown { .. })
    ));
    assert_eq!(
        owner
            .webhook(&event(123, "synthetic-mat", 51, 1002), 1002, 0, 0)
            .status,
        200
    );
    let key_a = presence_namespace("123", "synthetic-mat").unwrap();
    let key_b = presence_namespace("123", "matB").unwrap();
    let preserved = shared.0.borrow().values.clone();
    assert_eq!(preserved.len(), 2);
    assert!(preserved.contains_key(&key_a) && preserved.contains_key(&key_b));
    drop(owner);
    let mut owner = Coordinator::<ScopedStore<Shared>, Shared>::default();
    owner.begin_account(Some(123));
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1002)
        .unwrap();
    owner
        .discover(&mut transport, &mut selections, &record(123), 1002)
        .unwrap();
    shared.0.borrow_mut().fail_load = true;
    assert!(
        owner
            .activate_scoped(&mut selections, 123, shared.clone(), shared.clone())
            .is_err()
    );
    assert_eq!(
        owner
            .webhook(&event(123, "synthetic-mat", 51, 1002), 1002, 0, 0)
            .status,
        503
    );
    shared.0.borrow_mut().fail_load = false;
    owner
        .activate_scoped(&mut selections, 123, shared.clone(), shared.clone())
        .unwrap();
    assert_eq!(
        owner
            .webhook(&event(123, "synthetic-mat", 51, 1002), 1002, 0, 0)
            .status,
        200
    );
    assert!(matches!(
        shared.0.borrow().readings.last(),
        Some(Reading::Unknown { .. })
    ));
    shared.0.borrow_mut().fail_save = true;
    assert_eq!(
        owner
            .webhook(&event(123, "synthetic-mat", 50, 1003), 1003, 0, 0)
            .status,
        503
    );
    assert_eq!(shared.0.borrow().values, preserved);
    shared.0.borrow_mut().fail_save = false;
    owner.begin_account(Some(999));
    owner
        .restore_account(Some(&record(999).serialize().unwrap()), 999, 1003)
        .unwrap();
    owner
        .discover(&mut transport, &mut selections, &record(999), 1003)
        .unwrap();
    owner
        .accept_scoped(
            &mut selections,
            "synthetic-mat",
            owner.generation(),
            shared.clone(),
            shared.clone(),
        )
        .unwrap();
    assert_eq!(
        owner
            .webhook(&event(999, "synthetic-mat", 50, 1003), 1003, 0, 0)
            .status,
        200
    );
    let d = shared.0.borrow();
    assert_eq!(d.values.len(), 3);
    assert_eq!(d.values[&key_a], preserved[&key_a]);
    assert_eq!(d.values[&key_b], preserved[&key_b]);
    assert_eq!(d.actions, vec!["bed_in", "bed_in", "bed_out", "bed_in"]);
}
#[test]
fn coordinator_presentation_follows_owned_setup_states_and_redacts_actions() {
    use niles_withings::coordinator::{
        ConnectionState, Coordinator, DiscoveryState, OccupancyState, SetupAction,
        SubscriptionState,
    };
    let mut owner =
        Coordinator::<niles_withings::coordinator::ScopedStore<StateStore>, Events>::default();
    assert_eq!(
        owner.presentation().connection,
        ConnectionState::Disconnected
    );
    assert_eq!(owner.presentation().actions, vec![SetupAction::Connect]);
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
        .unwrap();
    owner.restore_account(None, 123, 1001).unwrap_err();
    assert_eq!(
        owner.presentation().discovery,
        DiscoveryState::AuthorizationRequired
    );
    assert_eq!(owner.presentation().actions, vec![SetupAction::Reconnect]);
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
        .unwrap();
    let mut selections = Selections::default();
    let mut transport = Transport {
        replies: [
            devices(MAT),
            devices(MAT),
            devices(MAT),
            HttpResponse {
                status: 503,
                body: vec![],
            },
        ]
        .into(),
        calls: 0,
    };
    owner
        .discover(&mut transport, &mut selections, &record(123), 1001)
        .unwrap();
    let view = owner.presentation();
    assert_eq!(view.discovery, DiscoveryState::Candidate);
    assert_eq!(view.occupancy, OccupancyState::Unknown);
    assert!(!format!("{view:?}").contains("synthetic-mat"));
    owner
        .accept_scoped(
            &mut selections,
            "synthetic-mat",
            owner.generation(),
            StateStore::default(),
            Events::default(),
        )
        .unwrap();
    assert_eq!(
        owner.presentation().subscription,
        SubscriptionState::Incomplete
    );
    owner
        .discover(&mut transport, &mut selections, &record(123), 1001)
        .unwrap();
    owner.set_lifecycle(niles_withings::lifecycle::Status::Ready);
    let view = owner.presentation();
    assert_eq!(view.discovery, DiscoveryState::Selected);
    assert_eq!(view.subscription, SubscriptionState::Ready);
    assert_eq!(view.occupancy, OccupancyState::Unknown);
    owner.switch_device();
    owner
        .discover(&mut transport, &mut selections, &record(123), 1001)
        .unwrap();
    assert_eq!(owner.presentation().discovery, DiscoveryState::Candidate);
    owner
        .discover(&mut transport, &mut selections, &record(123), 1001)
        .unwrap_err();
    assert_eq!(owner.presentation().discovery, DiscoveryState::Unavailable);
    assert_eq!(
        owner.presentation().actions,
        vec![SetupAction::RetryDiscovery]
    );
}

#[test]
fn presentation_multiple_empty_stale_and_pending() {
    use niles_withings::coordinator::{
        Coordinator, DiscoveryState, ScopedStore, SetupAction, SubscriptionState,
    };
    let mut owner = Coordinator::<ScopedStore<StateStore>, Events>::default();
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
        .unwrap();
    let mut selections = Selections::default();
    let rows = format!("{MAT},{}", MAT.replace("synthetic-mat", "matB"));
    let mut t = Transport {
        replies: [
            devices(""),
            devices(&rows),
            devices(&MAT.replace("synthetic-mat", "matB")),
        ]
        .into(),
        calls: 0,
    };
    owner
        .discover(&mut t, &mut selections, &record(123), 1001)
        .unwrap();
    assert_eq!(
        owner.presentation().discovery,
        DiscoveryState::NoEligibleMats
    );
    owner
        .discover(&mut t, &mut selections, &record(123), 1001)
        .unwrap();
    let v = owner.presentation();
    assert_eq!(v.discovery, DiscoveryState::MultipleCandidates);
    assert_eq!(v.candidates.len(), 2);
    assert_eq!(v.actions.len(), 2);
    assert!(v.actions.iter().all(|a| matches!(a,SetupAction::ChooseCandidate{generation,..} if *generation==owner.generation())));
    owner
        .accept_scoped(
            &mut selections,
            "synthetic-mat",
            owner.generation(),
            StateStore::default(),
            Events::default(),
        )
        .unwrap();
    let v = owner.presentation();
    assert_eq!(v.discovery, DiscoveryState::Selected);
    assert!(v.selected.is_some());
    owner.set_lifecycle(niles_withings::lifecycle::Status::Ready);
    owner.restore_account(None, 123, 1001).unwrap_err();
    assert_ne!(owner.presentation().subscription, SubscriptionState::Ready);
    assert!(owner.presentation().selected.is_none());
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
        .unwrap();
    owner
        .discover(&mut t, &mut selections, &record(123), 1001)
        .unwrap();
    let v = owner.presentation();
    assert_eq!(v.discovery, DiscoveryState::StaleSelection);
    assert!(v.selected.is_none());
    assert_eq!(v.actions, vec![SetupAction::SwitchMat]);
}
#[test]
fn restore_failure_revokes_delivery_and_requires_validated_recovery() {
    use niles_withings::{
        application::WebhookRequest,
        coordinator::{Coordinator, ScopedStore, SetupAction},
    };
    let valid = record(123).serialize().unwrap();
    let mut pending: serde_json::Value = serde_json::from_str(&valid).unwrap();
    pending["version"] = 2.into();
    pending["refresh_pending"] = true.into();
    let foreign = record(999).serialize().unwrap();
    for raw in [
        None,
        Some("broken".to_string()),
        Some(pending.to_string()),
        Some(foreign),
    ] {
        let mut c = Coordinator::<ScopedStore<Shared>, Shared>::default();
        let shared = Shared::default();
        let mut s = Selections::default();
        c.restore_account(Some(&valid), 123, 1001).unwrap();
        let mut t = Transport {
            replies: [devices(MAT), devices(MAT)].into(),
            calls: 0,
        };
        c.discover(&mut t, &mut s, &record(123), 1001).unwrap();
        c.accept_scoped(
            &mut s,
            "synthetic-mat",
            c.generation(),
            shared.clone(),
            shared.clone(),
        )
        .unwrap();
        let e = WebhookRequest {
            method: "POST".into(),
            content_type: "application/x-www-form-urlencoded".into(),
            body: b"userid=123&deviceid=synthetic-mat&appli=50&date=1001".to_vec(),
        };
        assert_eq!(c.webhook(&e, 1001, 0, 0).status, 200);
        c.restore_account(raw.as_deref(), 123, 1001).unwrap_err();
        c.begin_account(Some(123));
        assert_eq!(c.webhook(&e, 1001, 0, 0).status, 503);
        assert_eq!(c.presentation().actions, vec![SetupAction::Reconnect]);
        assert!(c.discover(&mut t, &mut s, &record(123), 1001).is_err());
        assert_eq!(t.calls, 1);
        c.restore_account(Some(&valid), 123, 1001).unwrap();
        c.discover(&mut t, &mut s, &record(123), 1001).unwrap();
        c.activate_scoped(&mut s, 123, shared.clone(), shared.clone())
            .unwrap();
        assert_eq!(c.webhook(&e, 1001, 0, 0).status, 200);
        assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
    }
}

#[test]
fn coordinator_discovery_requires_validated_startup_after_account_switches() {
    use niles_withings::coordinator::{Coordinator, ScopedStore};
    let mut owner = Coordinator::<ScopedStore<Shared>, Shared>::default();
    let mut selections = Selections::default();
    let mut transport = Transport {
        replies: [devices(MAT)].into(),
        calls: 0,
    };
    owner.begin_account(Some(123));
    assert!(
        owner
            .discover(&mut transport, &mut selections, &record(123), 1001)
            .is_err()
    );
    assert_eq!(transport.calls, 0);
    owner.begin_account(Some(999));
    owner.begin_account(Some(123));
    assert!(
        owner
            .discover(&mut transport, &mut selections, &record(123), 1001)
            .is_err()
    );
    assert_eq!(transport.calls, 0);
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
        .unwrap();
    owner
        .discover(&mut transport, &mut selections, &record(123), 1001)
        .unwrap();
    assert_eq!(transport.calls, 1);
}

#[test]
fn failed_restore_rejects_stale_discovery_record_before_provider_call() {
    use niles_withings::coordinator::{Coordinator, ScopedStore};
    let valid = record(123).serialize().unwrap();
    let mut pending: serde_json::Value = serde_json::from_str(&valid).unwrap();
    pending["version"] = 2.into();
    pending["refresh_pending"] = true.into();
    for raw in ["broken".to_string(), pending.to_string()] {
        let mut owner = Coordinator::<ScopedStore<Shared>, Shared>::default();
        owner.begin_account(Some(123));
        assert!(owner.restore_account(Some(&raw), 123, 1001).is_err());
        let mut transport = Transport {
            replies: [devices(MAT)].into(),
            calls: 0,
        };
        assert!(
            owner
                .discover(
                    &mut transport,
                    &mut Selections::default(),
                    &record(123),
                    1001
                )
                .is_err()
        );
        assert_eq!(transport.calls, 0);
        owner.restore_account(Some(&valid), 123, 1001).unwrap();
        owner
            .discover(
                &mut transport,
                &mut Selections::default(),
                &record(123),
                1001,
            )
            .unwrap();
        assert_eq!(transport.calls, 1);
    }
}
#[test]
fn callback_recovery_through_coordinator_to_bed_event() {
    use niles_withings::{
        application::{AccountSecretStore, SecureAccountRecord, WebhookRequest, on_callback},
        coordinator::{Coordinator, ScopedStore},
        withings_client::{CallbackUrl, Secret},
    };
    #[derive(Default)]
    struct Secrets(String);
    impl AccountSecretStore for Secrets {
        type Error = ();
        fn set_account_record(&mut self, s: &str) -> Result<(), ()> {
            self.0 = s.into();
            Ok(())
        }
    }
    let mut c = Coordinator::<ScopedStore<Shared>, Shared>::default();
    let mut pending: serde_json::Value =
        serde_json::from_str(&record(123).serialize().unwrap()).unwrap();
    pending["version"] = 2.into();
    pending["refresh_pending"] = true.into();
    c.restore_account(Some(&pending.to_string()), 123, 1001)
        .unwrap_err();
    let shared = Shared::default();
    let mut selections = Selections::default();
    let mut secrets = Secrets::default();
    struct FlowTransport {
        replies: std::collections::VecDeque<HttpResponse>,
        calls: usize,
    }
    impl HttpTransport for FlowTransport {
        type Error = ();
        fn post(&mut self, request: HttpRequest) -> Result<HttpResponse, ()> {
            if self.calls == 0 {
                assert!(request.form.starts_with("action=requesttoken&"));
            } else {
                assert_eq!(request.form, "action=getdevice");
            }
            self.calls += 1;
            self.replies.pop_front().ok_or(())
        }
    }
    let mut t=FlowTransport{replies:[HttpResponse{status:200,body:br#"{"status":0,"body":{"userid":123,"access_token":"callback-access","refresh_token":"callback-refresh","expires_in":3600,"scope":"user.sleepevents","token_type":"Bearer"}}"#.to_vec()},devices(MAT)].into(),calls:0};
    on_callback(
        &mut t,
        &mut secrets,
        &CallbackUrl::parse("https://example.com/callback").unwrap(),
        &[("code".into(), "synthetic".into())],
        None,
        "client",
        &Secret::new("secret").unwrap(),
        1001,
    )
    .unwrap();
    let (tokens, meta) = c.restore_account(Some(&secrets.0), 123, 1001).unwrap();
    let restored = SecureAccountRecord::from_tokens(&tokens, meta).unwrap();
    c.discover(&mut t, &mut selections, &restored, 1001)
        .unwrap();
    c.accept_scoped(
        &mut selections,
        "synthetic-mat",
        c.generation(),
        shared.clone(),
        shared.clone(),
    )
    .unwrap();
    assert!(shared.0.borrow().actions.is_empty());
    let e = WebhookRequest {
        method: "POST".into(),
        content_type: "application/x-www-form-urlencoded".into(),
        body: b"userid=123&deviceid=synthetic-mat&appli=50&date=1001".to_vec(),
    };
    assert_eq!(c.webhook(&e, 1001, 0, 0).status, 200);
    assert_eq!(c.webhook(&e, 1001, 0, 0).status, 200);
    assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
    assert_eq!(t.calls, 2);
}
#[test]
fn callback_recovery_through_runtime_to_bed_event() {
    use niles_withings::{
        application::{AccountSecretStore, WebhookRequest},
        withings_client::{CallbackUrl, Secret},
    };
    #[derive(Default)]
    struct Secrets(String);
    impl AccountSecretStore for Secrets {
        type Error = ();
        fn set_account_record(&mut self, s: &str) -> Result<(), ()> {
            self.0 = s.into();
            Ok(())
        }
    }
    let mut c = niles_withings::runtime::Owner::<Shared, Shared>::new();
    let mut pending: serde_json::Value =
        serde_json::from_str(&record(123).serialize().unwrap()).unwrap();
    pending["version"] = 2.into();
    pending["refresh_pending"] = true.into();
    let shared = Shared::default();
    let mut selections = Selections::default();
    impl niles_withings::runtime::AccountRecordRead for Secrets {
        fn read_account_record(&mut self) -> Result<Option<String>, ()> {
            Ok(Some(self.0.clone()))
        }
    }
    let mut secrets = Secrets(pending.to_string());
    c.restore(&mut secrets, 123, 1001).unwrap_err();
    struct FlowTransport {
        replies: std::collections::VecDeque<HttpResponse>,
        calls: usize,
    }
    impl HttpTransport for FlowTransport {
        type Error = ();
        fn post(&mut self, request: HttpRequest) -> Result<HttpResponse, ()> {
            if self.calls == 0 {
                assert!(request.form.starts_with("action=requesttoken&"));
            } else {
                assert_eq!(request.form, "action=getdevice");
            }
            self.calls += 1;
            self.replies.pop_front().ok_or(())
        }
    }
    let mut t=FlowTransport{replies:[HttpResponse{status:200,body:br#"{"status":0,"body":{"userid":123,"access_token":"callback-access","refresh_token":"callback-refresh","expires_in":3600,"scope":"user.sleepevents","token_type":"Bearer"}}"#.to_vec()},devices(MAT)].into(),calls:0};
    c.callback(
        &mut t,
        &mut secrets,
        &CallbackUrl::parse("https://example.com/callback").unwrap(),
        &[("code".into(), "synthetic".into())],
        None,
        "client",
        &Secret::new("secret").unwrap(),
        1001,
    )
    .unwrap();
    c.discover(&mut t, &mut selections, 1001).unwrap();
    c.accept(
        &mut selections,
        "synthetic-mat",
        c.generation(),
        shared.clone(),
        shared.clone(),
    )
    .unwrap();
    assert!(shared.0.borrow().actions.is_empty());
    let e = WebhookRequest {
        method: "POST".into(),
        content_type: "application/x-www-form-urlencoded".into(),
        body: b"userid=123&deviceid=synthetic-mat&appli=50&date=1001".to_vec(),
    };
    assert_eq!(c.webhook(&e, 1001, 0, 0).status, 200);
    assert_eq!(c.webhook(&e, 1001, 0, 0).status, 200);
    assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
    assert_eq!(t.calls, 2);
    let old_generation = c.generation();
    c.switch_mat();
    assert_eq!(c.webhook(&e, 1001, 0, 0).status, 503);
    t.replies.push_back(devices(MAT));
    c.discover(&mut t, &mut selections, 1001).unwrap();
    assert!(
        c.accept(
            &mut selections,
            "synthetic-mat",
            old_generation,
            shared.clone(),
            shared.clone()
        )
        .is_err()
    );
    c.accept(
        &mut selections,
        "synthetic-mat",
        c.generation(),
        shared.clone(),
        shared.clone(),
    )
    .unwrap();
    let saved = secrets.0.clone();
    let calls = t.calls;
    c.pause();
    assert_eq!(c.webhook(&e, 1001, 0, 0).status, 503);
    assert!(c.discover(&mut t, &mut selections, 1001).is_err());
    assert_eq!(t.calls, calls);
    assert_eq!(secrets.0, saved);
    c.restore(&mut secrets, 123, 1001).unwrap();
    t.replies.push_back(devices(MAT));
    c.discover(&mut t, &mut selections, 1001).unwrap();
    c.activate(&mut selections, 123, shared.clone(), shared.clone())
        .unwrap();
    assert_eq!(c.webhook(&e, 1001, 0, 0).status, 200);
    assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
    // An invalid callback must revoke an already-active owner without HTTP.
    let calls = t.calls;
    assert!(
        c.callback(
            &mut t,
            &mut secrets,
            &CallbackUrl::parse("https://example.com/callback").unwrap(),
            &[],
            None,
            "client",
            &Secret::new("secret").unwrap(),
            1001
        )
        .is_err()
    );
    assert_eq!(c.webhook(&e, 1001, 0, 0).status, 503);
    assert_eq!(t.calls, calls);
}
#[test]
fn runtime_restore_rotation_revocation_and_restart() {
    use niles_withings::{
        application::{AccountSecretStore, WebhookRequest},
        lifecycle::{FixedClock, Status},
        runtime::{AccountRecordRead, Owner},
        withings_client::{CallbackUrl, Secret},
    };
    struct Secrets {
        raw: String,
        writes: usize,
        fail: usize,
        read_fail: bool,
    }
    impl AccountSecretStore for Secrets {
        type Error = ();
        fn set_account_record(&mut self, s: &str) -> Result<(), ()> {
            self.writes += 1;
            if self.writes == self.fail {
                return Err(());
            }
            self.raw = s.into();
            Ok(())
        }
    }
    impl AccountRecordRead for Secrets {
        fn read_account_record(&mut self) -> Result<Option<String>, ()> {
            if self.read_fail {
                Err(())
            } else {
                Ok(Some(self.raw.clone()))
            }
        }
    }
    struct Http {
        replies: std::collections::VecDeque<HttpResponse>,
        bearers: Vec<Option<String>>,
    }
    impl HttpTransport for Http {
        type Error = ();
        fn post(&mut self, r: HttpRequest) -> Result<HttpResponse, ()> {
            self.bearers
                .push(r.bearer.map(|s| s.expose_secret().to_owned()));
            self.replies.pop_front().ok_or(())
        }
    }
    let token = || {
        HttpResponse{status:200,body:br#"{"status":0,"body":{"userid":123,"access_token":"rotated","refresh_token":"rotated-refresh","expires_in":3600,"scope":"user.sleepevents","token_type":"Bearer"}}"#.to_vec()}
    };
    let profiles = || {
        HttpResponse{status:200,body:br#"{"status":0,"body":{"profiles":[{"appli":50,"callbackurl":"https://example.com/hook","comment":"owned"},{"appli":51,"callbackurl":"https://example.com/hook","comment":"owned"}]}}"#.to_vec()}
    };
    let mut secrets = Secrets {
        raw: record(123).serialize().unwrap(),
        writes: 0,
        fail: usize::MAX,
        read_fail: false,
    };
    let mut h = Http {
        replies: [
            devices(MAT),
            token(),
            profiles(),
            profiles(),
            profiles(),
            profiles(),
            devices(MAT),
            devices(MAT),
            token(),
        ]
        .into(),
        bearers: vec![],
    };
    let mut o = Owner::<Shared, Shared>::new();
    let shared = Shared::default();
    let mut selections = Selections::default();
    assert!(o.discover(&mut h, &mut selections, 1001).is_err());
    assert!(h.bearers.is_empty());
    o.restore(&mut secrets, 123, 1001).unwrap();
    o.discover(&mut h, &mut selections, 1001).unwrap();
    o.accept(
        &mut selections,
        "synthetic-mat",
        o.generation(),
        shared.clone(),
        shared.clone(),
    )
    .unwrap();
    let event = |appli, date| WebhookRequest {
        method: "POST".into(),
        content_type: "application/x-www-form-urlencoded".into(),
        body: format!("userid=123&deviceid=synthetic-mat&appli={appli}&date={date}").into_bytes(),
    };
    assert_eq!(o.webhook(&event(50, 1001), 1001, 0, 0).status, 200);
    let url = CallbackUrl::parse("https://example.com/hook").unwrap();
    let secret = Secret::new("secret").unwrap();
    assert_eq!(
        o.reconcile(
            &mut h,
            &mut secrets,
            &FixedClock(4500),
            "client",
            &secret,
            &url,
            "owned"
        )
        .status,
        Status::Ready
    );
    o.discover(&mut h, &mut selections, 4500).unwrap();
    assert_eq!(h.bearers.last().unwrap().as_deref(), Some("rotated"));
    drop(o);
    let mut o = Owner::<Shared, Shared>::new();
    o.restore(&mut secrets, 123, 4500).unwrap();
    o.discover(&mut h, &mut selections, 4500).unwrap();
    o.activate(&mut selections, 123, shared.clone(), shared.clone())
        .unwrap();
    assert_eq!(o.webhook(&event(50, 1001), 4500, 10000, 0).status, 200);
    assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
    assert_eq!(o.webhook(&event(51, 4501), 4501, 0, 0).status, 200);
    secrets.fail = secrets.writes + 2;
    assert_eq!(
        o.reconcile(
            &mut h,
            &mut secrets,
            &FixedClock(8000),
            "client",
            &secret,
            &url,
            "owned"
        )
        .status,
        Status::RefreshPersistenceFailed
    );
    assert_eq!(o.webhook(&event(50, 8000), 8000, 0, 0).status, 503);
    assert!(o.restore(&mut secrets, 123, 8000).is_err());
    let calls = h.bearers.len();
    assert!(o.discover(&mut h, &mut selections, 8000).is_err());
    assert_eq!(h.bearers.len(), calls);
    secrets.read_fail = true;
    assert!(o.restore(&mut secrets, 123, 8000).is_err());
    assert_eq!(o.webhook(&event(50, 8000), 8000, 0, 0).status, 503);
    assert_eq!(shared.0.borrow().actions, vec!["bed_in", "bed_out"]);
    for failure in ["denied", "invalid", "save", "read"] {
        secrets.read_fail = false;
        secrets.fail = usize::MAX;
        secrets.raw = record(123).serialize().unwrap();
        o.restore(&mut secrets, 123, 1001).unwrap();
        h.replies.push_back(devices(MAT));
        o.discover(&mut h, &mut selections, 1001).unwrap();
        o.activate(&mut selections, 123, shared.clone(), shared.clone())
            .unwrap();
        if failure == "save" {
            secrets.fail = secrets.writes + 1;
        }
        if failure == "read" {
            secrets.read_fail = true;
        }
        if failure != "denied" {
            h.replies.push_back(if failure == "invalid" {
                HttpResponse {
                    status: 200,
                    body: b"invalid".to_vec(),
                }
            } else {
                token()
            });
        }
        let query = if failure == "denied" {
            vec![("error".into(), "access_denied".into())]
        } else {
            vec![("code".into(), "synthetic".into())]
        };
        let _ = o.callback(
            &mut h,
            &mut secrets,
            &url,
            &query,
            None,
            "client",
            &secret,
            1001,
        );
        assert_eq!(
            o.webhook(&event(50, 1001), 1001, 0, 0).status,
            503,
            "{failure}"
        );
        let calls = h.bearers.len();
        assert!(o.discover(&mut h, &mut selections, 1001).is_err());
        assert_eq!(
            o.reconcile(
                &mut h,
                &mut secrets,
                &FixedClock(1001),
                "client",
                &secret,
                &url,
                "owned"
            )
            .status,
            Status::AuthorizationRequired
        );
        assert_eq!(h.bearers.len(), calls);
    }
}
const MAT: &str =
    r#"{"deviceid":"synthetic-mat","type":"Sleep Monitor","model":"Aura Sensor","model_id":61}"#;

#[test]
fn coordinator_account_switch_stale_generation_and_failed_save() {
    use niles_withings::{
        application::WebhookRequest,
        coordinator::{Coordinator, SetupStatus},
    };
    let mut owner =
        Coordinator::<niles_withings::coordinator::ScopedStore<StateStore>, Events>::default();
    let request = |user| WebhookRequest {
        method: "POST".into(),
        content_type: "application/x-www-form-urlencoded".into(),
        body: format!("userid={user}&deviceid=synthetic-mat&appli=50&date=1001").into_bytes(),
    };
    let mut selections = Selections::default();
    let mut transport = Transport {
        replies: [
            devices(MAT),
            devices(MAT),
            devices(MAT),
            devices(MAT),
            devices(""),
        ]
        .into(),
        calls: 0,
    };
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
        .unwrap();
    owner
        .discover(&mut transport, &mut selections, &record(123), 1001)
        .unwrap();
    let old = owner.generation();
    owner
        .discover(&mut transport, &mut selections, &record(123), 1001)
        .unwrap();
    assert!(
        owner
            .accept_scoped(
                &mut selections,
                "synthetic-mat",
                old,
                StateStore::default(),
                Events::default()
            )
            .is_err()
    );
    assert!(selections.value.is_none());
    selections.fail_save = true;
    assert!(
        owner
            .accept_scoped(
                &mut selections,
                "synthetic-mat",
                owner.generation(),
                StateStore::default(),
                Events::default()
            )
            .is_err()
    );
    assert_eq!(owner.webhook(&request(123), 1001, 0, 0).status, 503);
    selections.fail_save = false;
    owner
        .accept_scoped(
            &mut selections,
            "synthetic-mat",
            owner.generation(),
            StateStore::default(),
            Events::default(),
        )
        .unwrap();
    assert_eq!(owner.webhook(&request(123), 1001, 0, 0).status, 200);
    owner.set_lifecycle(niles_withings::lifecycle::Status::Ready);
    owner.begin_account(Some(123));
    assert_eq!(owner.status(), SetupStatus::Configured);
    owner.begin_account(Some(999));
    assert_ne!(owner.status(), SetupStatus::Configured);
    owner
        .restore_account(Some(&record(999).serialize().unwrap()), 999, 1001)
        .unwrap();
    assert_eq!(owner.webhook(&request(123), 1001, 0, 0).status, 503);
    assert!(
        owner
            .activate_scoped(
                &mut selections,
                999,
                StateStore::default(),
                Events::default()
            )
            .is_err()
    );
    owner
        .discover(&mut transport, &mut selections, &record(999), 1001)
        .unwrap();
    owner
        .accept_scoped(
            &mut selections,
            "synthetic-mat",
            owner.generation(),
            StateStore::default(),
            Events::default(),
        )
        .unwrap();
    assert_eq!(owner.webhook(&request(123), 1001, 0, 0).status, 400);
    assert_eq!(owner.webhook(&request(999), 1001, 0, 0).status, 200);
    owner
        .discover(&mut transport, &mut selections, &record(999), 1001)
        .unwrap();
    owner
        .activate_scoped(
            &mut selections,
            999,
            StateStore::default(),
            Events::default(),
        )
        .unwrap();
    owner
        .discover(&mut transport, &mut selections, &record(999), 1001)
        .unwrap();
    assert_eq!(owner.status(), SetupStatus::SelectionStale);
    assert!(
        owner
            .activate_scoped(
                &mut selections,
                999,
                StateStore::default(),
                Events::default()
            )
            .is_err()
    );
    assert_eq!(owner.webhook(&request(999), 1001, 0, 0).status, 503);
    owner.begin_account(None);
    owner.discovery_failed();
    assert_eq!(owner.status(), SetupStatus::AccountRequired);
}

#[test]
fn coordinator_scoped_presence_namespace_encoding_is_unambiguous_and_validated() {
    use niles_withings::coordinator::presence_namespace;
    assert_eq!(
        presence_namespace("12", "3").unwrap(),
        "niles-withings:presence:v1:2:12:1:3"
    );
    assert_eq!(
        presence_namespace("1", "23").unwrap(),
        "niles-withings:presence:v1:1:1:2:23"
    );
    assert_ne!(
        presence_namespace("12", "3").unwrap(),
        presence_namespace("1", "23").unwrap()
    );
    assert!(presence_namespace("", "matA").is_err());
    assert!(presence_namespace("123", "bad\nmat").is_err());
}

#[test]
fn selection_is_explicit_durable_and_gates_adapter_events() {
    let mut transport = Transport {
        replies: std::collections::VecDeque::from([devices(MAT), devices(MAT)]),
        calls: 0,
    };
    let mut selections = Selections::default();
    let account = record(123);
    let candidate = setup_discover(&mut transport, &mut selections, &account, 123, 1001).unwrap();
    assert_eq!(
        candidate.presentation(),
        SetupPresentation::CandidateAvailable
    );
    assert!(accept_selection(&mut selections, 123, &candidate, "other").is_err());
    assert!(accept_selection(&mut selections, 999, &candidate, "synthetic-mat").is_err());
    assert!(selections.value.is_none());
    let binding = accept_selection(&mut selections, 123, &candidate, "synthetic-mat").unwrap();
    assert!(selections.value.is_some());
    let selected = setup_discover(&mut transport, &mut selections, &account, 123, 1001).unwrap();
    assert!(matches!(selected, SetupResolution::Selected(_)));

    let mut adapter =
        adapter_from_binding(&binding, StateStore::default(), Events::default()).unwrap();
    assert!(
        adapter
            .deliver(
                b"userid=999&deviceid=synthetic-mat&appli=50&date=1001",
                1001,
                0,
                0
            )
            .is_err()
    );
    adapter
        .deliver(
            b"userid=123&deviceid=synthetic-mat&appli=50&date=1001",
            1001,
            0,
            0,
        )
        .unwrap();

    selections.fail_save = true;
    let replacement = SetupResolution::Candidate {
        account: 123,
        device: niles_withings::withings_client::Device {
            device_id: "new-mat".into(),
            device_type: "Sleep Monitor".into(),
            model: "Synthetic".into(),
            model_id: 1,
        },
    };
    assert_eq!(
        accept_selection(&mut selections, 123, &replacement, "new-mat"),
        Err(ApplicationError::Storage)
    );
    assert!(
        selections
            .value
            .as_deref()
            .unwrap()
            .contains("synthetic-mat")
    );
}

#[test]
fn corrupt_foreign_and_stale_selection_fail_closed_without_switching() {
    let account = record(123);
    let mut transport = Transport {
        replies: std::collections::VecDeque::from([devices(MAT)]),
        calls: 0,
    };
    let mut selections = Selections {
        value: Some("not-json".into()),
        fail_save: false,
    };
    assert!(matches!(
        setup_discover(&mut transport, &mut selections, &account, 123, 1001),
        Err(ApplicationError::InvalidSelection)
    ));
    assert_eq!(transport.calls, 0);

    let mut foreign = Selections {
        value: Some(r#"{"version":1,"user_id":999,"device_id":"old"}"#.into()),
        fail_save: false,
    };
    let mut transport = Transport {
        replies: std::collections::VecDeque::from([devices(MAT)]),
        calls: 0,
    };
    assert!(matches!(
        setup_discover(&mut transport, &mut foreign, &account, 123, 1001).unwrap(),
        SetupResolution::Candidate { .. }
    ));

    let mut stale = Selections {
        value: Some(r#"{"version":1,"user_id":123,"device_id":"vanished"}"#.into()),
        fail_save: false,
    };
    let mut transport = Transport {
        replies: std::collections::VecDeque::from([devices(MAT)]),
        calls: 0,
    };
    assert!(matches!(
        setup_discover(&mut transport, &mut stale, &account, 123, 1001).unwrap(),
        SetupResolution::StaleSelection
    ));
}

#[test]
fn unsafe_lifecycle_revokes_adapter_and_ready_cannot_restore_it() {
    use niles_withings::{
        application::WebhookRequest,
        coordinator::{Coordinator, DiscoveryState, ScopedStore, SetupAction},
        lifecycle::Status,
    };
    for status in [
        Status::AuthorizationRequired,
        Status::RefreshPersistenceFailed,
    ] {
        let mut owner = Coordinator::<ScopedStore<Shared>, Shared>::default();
        let shared = Shared::default();
        let mut selections = Selections::default();
        let mut transport = Transport {
            replies: [devices(MAT), devices(MAT)].into(),
            calls: 0,
        };
        let event = WebhookRequest {
            method: "POST".into(),
            content_type: "application/x-www-form-urlencoded".into(),
            body: b"userid=123&deviceid=synthetic-mat&appli=50&date=1001".to_vec(),
        };
        owner
            .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
            .unwrap();
        owner
            .discover(&mut transport, &mut selections, &record(123), 1001)
            .unwrap();
        owner
            .accept_scoped(
                &mut selections,
                "synthetic-mat",
                owner.generation(),
                shared.clone(),
                shared.clone(),
            )
            .unwrap();
        owner.set_lifecycle(Status::Ready);
        assert_eq!(owner.webhook(&event, 1001, 0, 0).status, 200);
        let stored = shared.0.borrow().values.clone();
        let selected = selections.load_selection().unwrap();
        owner.set_lifecycle(status);
        assert_eq!(shared.0.borrow().values, stored);
        assert_eq!(selections.load_selection().unwrap(), selected);
        assert_eq!(owner.webhook(&event, 1001, 0, 0).status, 503);
        let view = owner.presentation();
        assert_eq!(view.discovery, DiscoveryState::AuthorizationRequired);
        assert!(
            view.actions
                .iter()
                .any(|action| matches!(action, SetupAction::Reconnect))
        );
        owner.set_lifecycle(Status::Ready);
        assert_eq!(owner.webhook(&event, 1001, 0, 0).status, 503);
        assert!(
            owner
                .discover(&mut transport, &mut selections, &record(123), 1001)
                .is_err()
        );
        owner
            .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
            .unwrap();
        owner
            .discover(&mut transport, &mut selections, &record(123), 1001)
            .unwrap();
        owner
            .activate_scoped(&mut selections, 123, shared.clone(), shared.clone())
            .unwrap();
        owner.set_lifecycle(Status::Ready);
        assert_eq!(owner.webhook(&event, 1001, 0, 0).status, 200);
        assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
    }
}

#[test]
fn incomplete_subscriptions_preserve_valid_webhook_handler() {
    use niles_withings::{
        application::WebhookRequest,
        coordinator::{Coordinator, ScopedStore, SubscriptionState},
        lifecycle::Status,
    };
    let mut owner = Coordinator::<ScopedStore<StateStore>, Events>::default();
    let mut selections = Selections::default();
    let mut transport = Transport {
        replies: [devices(MAT)].into(),
        calls: 0,
    };
    let event = WebhookRequest {
        method: "POST".into(),
        content_type: "application/x-www-form-urlencoded".into(),
        body: b"userid=123&deviceid=synthetic-mat&appli=50&date=1001".to_vec(),
    };
    owner
        .restore_account(Some(&record(123).serialize().unwrap()), 123, 1001)
        .unwrap();
    owner
        .discover(&mut transport, &mut selections, &record(123), 1001)
        .unwrap();
    owner
        .accept_scoped(
            &mut selections,
            "synthetic-mat",
            owner.generation(),
            StateStore::default(),
            Events::default(),
        )
        .unwrap();
    owner.set_lifecycle(Status::Ready);
    owner.set_lifecycle(Status::SubscriptionsIncomplete);
    assert_eq!(
        owner.presentation().subscription,
        SubscriptionState::Incomplete
    );
    assert_eq!(owner.webhook(&event, 1001, 0, 0).status, 200);
}
