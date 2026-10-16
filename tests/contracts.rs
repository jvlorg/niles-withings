use niles_withings::{
    config::Identity,
    presence::{Health, Presence, State},
    provider::{self, Direction, Event},
};
fn id() -> Identity {
    Identity::new("12345", "mat").unwrap()
}
fn body(appli: &str, date: &str) -> String {
    format!("userid=12345&deviceid=mat&appli={appli}&date={date}")
}
fn parse(s: &str) -> Result<Event, provider::ParseError> {
    provider::parse(s.as_bytes(), &id(), 1000, 300, 30)
}
fn event(t: i64, d: Direction) -> Event {
    Event {
        user_id: "12345".into(),
        device_id: "mat".into(),
        timestamp: t,
        direction: d,
    }
}
#[test]
fn valid_both_directions() {
    assert_eq!(parse(&body("50", "1000")).unwrap().direction, Direction::In);
    assert_eq!(
        parse(&body("51", "1000")).unwrap().direction,
        Direction::Out
    );
}
#[test]
fn required_fields() {
    for field in ["userid", "deviceid", "appli", "date"] {
        let s = body("50", "1000");
        let filtered = s
            .split('&')
            .filter(|x| !x.starts_with(&format!("{field}=")))
            .collect::<Vec<_>>()
            .join("&");
        assert!(parse(&filtered).is_err());
    }
}
#[test]
fn duplicate_decoded_fields() {
    for extra in [
        "userid=12345",
        "%75serid=12345",
        "deviceid=mat",
        "appli=50",
        "date=1000",
    ] {
        assert!(parse(&format!("{}&{extra}", body("50", "1000"))).is_err());
    }
}
#[test]
fn malformed_encoding() {
    for extra in ["x=%", "x=%G0", "x=%FF", "broken"] {
        assert!(parse(&format!("{}&{extra}", body("50", "1000"))).is_err());
    }
    assert!(provider::parse(&[255], &id(), 1000, 300, 30).is_err());
}
#[test]
fn percent_and_plus() {
    let x = Identity::new("12345", "mat one+").unwrap();
    let s = b"userid=12345&deviceid=mat+one%2B&appli=50&date=1000";
    assert!(provider::parse(s, &x, 1000, 300, 30).is_ok());
    assert_eq!(provider::form_encode("mat one+"), "mat+one%2B");
}
#[test]
fn wrong_identity() {
    for s in [
        body("50", "1000").replace("12345", "12346"),
        body("50", "1000").replace("mat", "other"),
    ] {
        assert!(parse(&s).is_err());
    }
}
#[test]
fn invalid_category_date() {
    for a in ["1", "44", "52", "255", "256", "x"] {
        assert!(parse(&body(a, "1000")).is_err());
    }
    for d in ["0", "-1", "x", "9223372036854775808"] {
        assert!(parse(&body("50", d)).is_err());
    }
}
#[test]
fn time_boundaries() {
    for d in [700, 1000, 1030] {
        assert!(parse(&body("50", &d.to_string())).is_ok());
    }
    for d in [699, 1031] {
        assert!(parse(&body("50", &d.to_string())).is_err());
    }
}
#[test]
fn body_limit() {
    let prefix = format!("{}&x=", body("50", "1000"));
    let s = format!("{prefix}{}", "a".repeat(4096 - prefix.len()));
    assert!(parse(&s).is_ok());
    assert!(parse(&(s + "a")).is_err());
}
#[test]
fn negative_policy_rejected() {
    for (age, skew) in [(-1, 30), (300, -1)] {
        assert!(provider::parse(body("50", "1000").as_bytes(), &id(), 1000, age, skew).is_err());
    }
}
#[test]
fn startup_first_and_repeat() {
    let mut s = State::new(id());
    assert_eq!(s.presence, Presence::Unknown);
    assert_eq!(s.apply(&event(1000, Direction::In)).action, Some("bed_in"));
    assert_eq!(s.apply(&event(1001, Direction::In)).action, None);
    assert_eq!(
        s.apply(&event(1002, Direction::Out)).action,
        Some("bed_out")
    );
}
#[test]
fn duplicates_and_older_do_not_mutate() {
    let mut s = State::new(id());
    s.apply(&event(1000, Direction::In));
    let before = s.clone();
    for e in [event(1000, Direction::In), event(999, Direction::Out)] {
        assert_eq!(s.apply(&e).action, None);
        assert_eq!(s, before);
    }
}
#[test]
fn equal_time_conflict_and_recovery() {
    let mut s = State::new(id());
    s.apply(&event(1000, Direction::In));
    let out = s.apply(&event(1000, Direction::Out));
    assert!(out.uncertain);
    assert_eq!(out.action, None);
    assert_eq!(s.health, Health::Uncertain);
    assert_eq!(s.apply(&event(1000, Direction::In)).action, None);
    assert_eq!(s.health, Health::Uncertain);
    s.apply(&event(1001, Direction::In));
    assert_eq!(s.health, Health::Confirmed);
}
#[test]
fn restart_dedup_and_new_confirmation() {
    let mut s = State::new(id());
    s.apply(&event(1000, Direction::In));
    let mut r = State::restore_for(&s.persist(), &id()).unwrap();
    assert_eq!(r.health, Health::Unconfirmed);
    assert_eq!(r.apply(&event(1000, Direction::In)).action, None);
    assert_eq!(r.health, Health::Unconfirmed);
    assert_eq!(r.apply(&event(1001, Direction::In)).action, None);
    assert_eq!(r.health, Health::Confirmed);
}
#[test]
fn cross_profile_restore() {
    let s = State::new(id());
    assert!(State::restore_for(&s.persist(), &Identity::new("999", "mat").unwrap()).is_err());
}
#[test]
fn corrupt_and_large_persistence() {
    for s in ["{".to_string(), "x".repeat(65537)] {
        assert!(State::restore(&s).is_err());
    }
    let mut s = State::new(id());
    s.cursor = Some(1000);
    assert!(State::restore(&s.persist()).is_err());
}
#[test]
fn forged_event_cannot_change_state() {
    let mut s = State::new(id());
    let before = s.clone();
    for e in [
        event(0, Direction::In),
        Event {
            user_id: "other".into(),
            ..event(1000, Direction::In)
        },
    ] {
        assert_eq!(s.apply(&e).action, None);
        assert_eq!(s, before);
    }
}
#[test]
fn long_occupancy_no_synthetic_exit() {
    let mut s = State::new(id());
    s.apply(&event(1000, Direction::In));
    assert_eq!(s.apply(&event(100000, Direction::In)).action, None);
    assert_eq!(s.presence, Presence::InBed);
}
