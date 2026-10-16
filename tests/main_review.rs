use niles_withings::{
    config::Identity,
    presence::{Health, State},
    provider,
};
#[test]
fn restored_observation_is_not_fresh_confirmation() {
    let id = Identity::new("12345", "synthetic-mat").unwrap();
    let mut state = State::new(id.clone());
    let event = provider::parse(
        b"userid=12345&deviceid=synthetic-mat&appli=50&date=2000000000",
        &id,
        2000000000,
        300,
        30,
    )
    .unwrap();
    assert_eq!(state.apply(&event).action, Some("bed_in"));
    let restored = State::restore(&state.persist()).unwrap();
    assert_ne!(
        restored.health,
        Health::Confirmed,
        "restart must not assert fresh confirmation from persisted history"
    );
}
#[test]
fn restored_identity_must_be_validated() {
    let mut state = State::new(Identity::new("12345", "synthetic-mat").unwrap());
    state.identity.device_id = String::new();
    assert!(
        State::restore(&state.persist()).is_err(),
        "serde bypasses constructor validation"
    );
}

#[test]
fn secure_record_discovery_checks_account_before_provider_and_adapter_identity() {
    use niles_withings::{
        application::{SecureAccountRecord, discover_account_devices},
        lifecycle::TokenMetadata,
        withings_client::{
            DeviceSelection, HttpRequest, HttpResponse, HttpTransport, Secret, TokenSet,
        },
    };
    struct Transport(usize);
    impl HttpTransport for Transport {
        type Error = ();
        fn post(&mut self, request: HttpRequest) -> Result<HttpResponse, ()> {
            self.0 += 1;
            assert_eq!(request.form, "action=getdevice");
            Ok(HttpResponse { status: 200, body: br#"{"status":0,"body":{"devices":[{"deviceid":"synthetic-mat","type":"Sleep Monitor","model":"Aura Sensor","model_id":61}]}}"#.to_vec() })
        }
    }
    let tokens = TokenSet {
        user_id: 12345,
        access_token: Secret::new("synthetic-access").unwrap(),
        refresh_token: Secret::new("synthetic-refresh").unwrap(),
        expires_in: 3600,
    };
    let record = SecureAccountRecord::from_tokens(
        &tokens,
        TokenMetadata::new_for_account(1000, 3600).unwrap(),
    )
    .unwrap();
    let mut transport = Transport(0);
    assert_eq!(
        discover_account_devices(&mut transport, &record, 999, Some("synthetic-mat"), 1001)
            .unwrap(),
        DeviceSelection::AccountMismatch
    );
    assert_eq!(
        transport.0, 0,
        "foreign account must not query using previous account token"
    );
    assert!(matches!(
        discover_account_devices(&mut transport, &record, 12345, None, 1001).unwrap(),
        DeviceSelection::Candidate(_)
    ));
    assert_eq!(transport.0, 1);
    for now in [999, 4600, 9000] {
        assert!(discover_account_devices(&mut transport, &record, 12345, None, now).is_err());
    }
    assert_eq!(
        transport.0, 1,
        "invalid/expired token must not reach provider"
    );
    let identity = Identity::new(record.user_id().to_string(), "synthetic-mat").unwrap();
    assert_eq!(identity.user_id, "12345");
}
