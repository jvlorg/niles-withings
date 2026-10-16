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
