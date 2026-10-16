use niles_withings::{
    config::Identity,
    presence::{Health, Presence, State},
    provider,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: withings-replay <new synthetic-state-file>")?;
    let id = Identity::new("12345", "synthetic-mat")?;
    let mut state = State::new(id.clone());
    let mut actions = Vec::new();
    for (category, time, expected) in [
        (50, 2000000000, Some("bed_in")),
        (50, 2000000000, None),
        (51, 1999999999, None),
        (50, 2000000010, None),
        (51, 2000000100, Some("bed_out")),
    ] {
        let body = format!("userid=12345&deviceid=synthetic-mat&appli={category}&date={time}");
        let event = provider::parse(body.as_bytes(), &id, 2000000100, 300, 30)?;
        let outcome = state.apply(&event);
        assert_eq!(outcome.action, expected);
        if let Some(action) = outcome.action {
            actions.push(action);
        }
        println!(
            "fixture category={category} time={time} presence={:?} action={:?}",
            state.presence, outcome.action
        );
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(state.persist().as_bytes())?;
    file.sync_all()?;
    drop(file);
    let mut restored = State::restore_for(&std::fs::read_to_string(&path)?, &id)?;
    assert_eq!(restored.health, Health::Unconfirmed);
    assert_eq!(restored.presence, Presence::OutOfBed);
    let replay = provider::parse(
        b"userid=12345&deviceid=synthetic-mat&appli=51&date=2000000100",
        &id,
        2000000100,
        300,
        30,
    )?;
    assert_eq!(restored.apply(&replay).action, None);
    assert_eq!(restored.health, Health::Unconfirmed);
    assert!(
        provider::parse(
            b"userid=999&deviceid=synthetic-mat&appli=50&date=2000000100",
            &id,
            2000000100,
            300,
            30
        )
        .is_err()
    );
    assert_eq!(actions, vec!["bed_in", "bed_out"]);
    println!(
        "PASS: fixture actions bed_in,bed_out; file restart unconfirmed; duplicate silent; foreign profile rejected. No provider or Niles host contacted."
    );
    Ok(())
}
