use niles_withings::{
    adapter::{Adapter, DeliveryError, Reading, Sink, Store},
    config::Identity,
};
use std::{cell::RefCell, rc::Rc};
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Data>>);
#[derive(Default)]
struct Data {
    state: Option<String>,
    calls: Vec<String>,
    fail_save: bool,
    fail_action: bool,
    fail_reading: bool,
    reading_error: &'static str,
}
impl Store for Shared {
    type Error = &'static str;
    fn load(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(self.0.borrow().state.clone())
    }
    fn save(&mut self, s: &str) -> Result<(), Self::Error> {
        let mut d = self.0.borrow_mut();
        d.calls.push("save".into());
        if d.fail_save {
            return Err("disk");
        };
        d.state = Some(s.into());
        Ok(())
    }
}
impl Sink for Shared {
    type Error = &'static str;
    fn publish_reading(&mut self, r: Reading) -> Result<(), Self::Error> {
        let mut d = self.0.borrow_mut();
        d.calls.push(format!("reading:{r:?}"));
        if d.fail_reading {
            return Err(d.reading_error);
        }
        Ok(())
    }
    fn publish_action(&mut self, a: &'static str) -> Result<(), Self::Error> {
        let mut d = self.0.borrow_mut();
        d.calls.push(format!("action:{a}"));
        if d.fail_action {
            return Err("ambiguous host result");
        };
        Ok(())
    }
}
fn id() -> Identity {
    Identity::new("12345", "mat").unwrap()
}
fn event(cat: u8, t: i64) -> String {
    format!("userid=12345&deviceid=mat&appli={cat}&date={t}")
}
#[test]
fn persists_before_actions_and_duplicate_restart_is_silent() {
    let shared = Shared::default();
    let mut a = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    a.deliver(event(50, 1000).as_bytes(), 1000, 100, 0).unwrap();
    assert_eq!(
        &shared.0.borrow().calls[..],
        &[
            "save",
            "reading:Occupied { occupied: true, observed_at: 1000 }",
            "action:bed_in"
        ]
    );
    let mut b = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    b.deliver(event(50, 1000).as_bytes(), 1000, 100, 0).unwrap();
    assert_eq!(
        shared
            .0
            .borrow()
            .calls
            .iter()
            .filter(|x| x.as_str() == "action:bed_in")
            .count(),
        1
    );
}
#[test]
fn storage_failure_emits_nothing_and_does_not_advance_state() {
    let shared = Shared::default();
    shared.0.borrow_mut().fail_save = true;
    let mut a = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    assert!(matches!(
        a.deliver(event(50, 1000).as_bytes(), 1000, 100, 0),
        Err(DeliveryError::Storage)
    ));
    assert_eq!(&shared.0.borrow().calls[..], &["save"]);
    assert_eq!(a.state().last_observation, None);
}
#[test]
fn action_sink_failure_is_reported_as_ambiguous_after_commit() {
    let shared = Shared::default();
    shared.0.borrow_mut().fail_action = true;
    let mut a = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    assert!(matches!(
        a.deliver(event(50, 1000).as_bytes(), 1000, 100, 0),
        Err(DeliveryError::ActionSink {
            action: "bed_in",
            ..
        })
    ));
    assert!(shared.0.borrow().state.is_some());
    assert!(shared.0.borrow().calls.iter().any(|x| x == "action:bed_in"));
}
#[test]
fn malformed_input_has_no_side_effects() {
    let shared = Shared::default();
    let mut a = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    assert!(a.deliver(b"bad", 1000, 100, 0).is_err());
    assert!(shared.0.borrow().calls.is_empty());
}
#[test]
fn unknown_and_known_mapping_is_not_numeric_absence() {
    let shared = Shared::default();
    let mut a = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    a.deliver(event(50, 1000).as_bytes(), 1000, 100, 0).unwrap();
    assert!(
        shared
            .0
            .borrow()
            .calls
            .iter()
            .any(|x| x == "reading:Occupied { occupied: true, observed_at: 1000 }")
    );
    let unknown = Adapter::new(id(), Shared::default(), Shared::default()).unwrap();
    assert_eq!(
        unknown.state().presence,
        niles_withings::presence::Presence::Unknown
    );
}

#[test]
fn restored_conflicting_event_publishes_unknown_without_an_action() {
    let shared = Shared::default();
    let mut first = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    first
        .deliver(event(50, 1000).as_bytes(), 1000, 100, 0)
        .unwrap();
    shared.0.borrow_mut().calls.clear();
    let mut restored = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    restored
        .deliver(event(51, 1000).as_bytes(), 1000, 100, 0)
        .unwrap();
    assert_eq!(
        restored.state().presence,
        niles_withings::presence::Presence::InBed
    );
    assert_eq!(
        restored.state().health,
        niles_withings::presence::Health::Uncertain
    );
    assert!(
        shared
            .0
            .borrow()
            .calls
            .iter()
            .any(|x| x.starts_with("reading:Unknown"))
    );
    assert!(
        !shared
            .0
            .borrow()
            .calls
            .iter()
            .any(|x| x.starts_with("action:"))
    );
}

#[test]
fn reading_sink_failure_is_redacted_after_committing() {
    let shared = Shared::default();
    {
        let mut d = shared.0.borrow_mut();
        d.fail_reading = true;
        d.reading_error = "PRIVATE_READING_SENTINEL";
    }
    let mut a = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    let error = a
        .deliver(event(50, 1000).as_bytes(), 1000, 100, 0)
        .unwrap_err();
    assert_eq!(error.to_string(), "ReadingSink");
    assert!(!format!("{error:?}").contains("PRIVATE_READING_SENTINEL"));
    assert!(shared.0.borrow().state.is_some());
    assert!(
        !shared
            .0
            .borrow()
            .calls
            .iter()
            .any(|x| x.starts_with("action:"))
    );
}

#[test]
fn storage_and_action_sentinels_are_not_exposed() {
    let shared = Shared::default();
    shared.0.borrow_mut().fail_save = true;
    let mut a = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    let error = a
        .deliver(event(50, 1000).as_bytes(), 1000, 100, 0)
        .unwrap_err();
    assert_eq!(error.to_string(), "Storage");
    assert!(!format!("{error:?}").contains("disk"));
    let shared = Shared::default();
    shared.0.borrow_mut().fail_action = true;
    let mut a = Adapter::new(id(), shared.clone(), shared.clone()).unwrap();
    let error = a
        .deliver(event(50, 1000).as_bytes(), 1000, 100, 0)
        .unwrap_err();
    assert!(!format!("{error:?}").contains("ambiguous host result"));
}
