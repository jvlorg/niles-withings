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
const MAT: &str =
    r#"{"deviceid":"synthetic-mat","type":"Sleep Monitor","model":"Aura Sensor","model_id":61}"#;

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
