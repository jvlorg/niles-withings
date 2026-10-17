pub mod config {
    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct Identity {
        pub user_id: String,
        pub device_id: String,
    }
    impl Identity {
        pub fn new(
            user_id: impl Into<String>,
            device_id: impl Into<String>,
        ) -> Result<Self, &'static str> {
            let user_id = user_id.into();
            let device_id = device_id.into();
            if user_id.is_empty()
                || device_id.is_empty()
                || user_id.len() > 128
                || device_id.len() > 128
            {
                return Err("invalid configured identity");
            }
            Ok(Self { user_id, device_id })
        }
    }
}
pub mod provider {
    use std::collections::BTreeMap;
    use std::fmt;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Direction {
        In,
        Out,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Event {
        pub user_id: String,
        pub device_id: String,
        pub timestamp: i64,
        pub direction: Direction,
    }
    #[derive(Debug, PartialEq, Eq)]
    pub enum ParseError {
        TooLarge,
        Malformed,
        DuplicateRequired,
        MissingRequired,
        InvalidValue,
    }
    impl fmt::Display for ParseError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "invalid Withings notification ({self:?})")
        }
    }
    impl std::error::Error for ParseError {}
    const MAX_BODY: usize = 4096;
    fn decode(s: &str) -> Result<String, ParseError> {
        let b = s.as_bytes();
        let mut o = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'+' {
                o.push(b' ');
                i += 1
            } else if b[i] == b'%' {
                if i + 2 >= b.len() {
                    return Err(ParseError::Malformed);
                }
                let h = |x: u8| match x {
                    b'0'..=b'9' => Some(x - b'0'),
                    b'a'..=b'f' => Some(x - b'a' + 10),
                    b'A'..=b'F' => Some(x - b'A' + 10),
                    _ => None,
                };
                o.push(
                    h(b[i + 1]).ok_or(ParseError::Malformed)? * 16
                        + h(b[i + 2]).ok_or(ParseError::Malformed)?,
                );
                i += 3
            } else {
                o.push(b[i]);
                i += 1
            }
        }
        String::from_utf8(o).map_err(|_| ParseError::Malformed)
    }
    pub fn parse(
        body: &[u8],
        id: &crate::config::Identity,
        now: i64,
        max_age: i64,
        future_skew: i64,
    ) -> Result<Event, ParseError> {
        if max_age < 0 || future_skew < 0 || now <= 0 {
            return Err(ParseError::InvalidValue);
        }
        if body.len() > MAX_BODY {
            return Err(ParseError::TooLarge);
        }
        let s = std::str::from_utf8(body).map_err(|_| ParseError::Malformed)?;
        let mut f = BTreeMap::new();
        for p in s.split('&') {
            if p.is_empty() {
                continue;
            }
            let (k, v) = p.split_once('=').ok_or(ParseError::Malformed)?;
            let (k, v) = (decode(k)?, decode(v)?);
            if f.contains_key(&k) && matches!(k.as_str(), "userid" | "appli" | "date" | "deviceid")
            {
                return Err(ParseError::DuplicateRequired);
            }
            f.insert(k, v);
        }
        let get = |k: &str| f.get(k).ok_or(ParseError::MissingRequired);
        let user = get("userid")?;
        let device = get("deviceid")?;
        if user != &id.user_id || device != &id.device_id {
            return Err(ParseError::InvalidValue);
        }
        let direction = match get("appli")?
            .parse::<u8>()
            .map_err(|_| ParseError::InvalidValue)?
        {
            50 => Direction::In,
            51 => Direction::Out,
            _ => return Err(ParseError::InvalidValue),
        };
        let timestamp = get("date")?
            .parse::<i64>()
            .map_err(|_| ParseError::InvalidValue)?;
        if timestamp <= 0
            || timestamp < now.saturating_sub(max_age)
            || timestamp > now.saturating_add(future_skew)
        {
            return Err(ParseError::InvalidValue);
        }
        Ok(Event {
            user_id: user.clone(),
            device_id: device.clone(),
            timestamp,
            direction,
        })
    }
    pub fn form_encode(s: &str) -> String {
        let mut o = String::new();
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || b"*-._".contains(&b) {
                o.push(b as char)
            } else if b == b' ' {
                o.push('+')
            } else {
                o.push_str(&format!("%{b:02X}"))
            }
        }
        o
    }
}

/// Offline-testable OAuth and notification API contracts. This module is not
/// wired into the Niles component: the pinned host has no secret-store API.
pub mod withings_client;

/// Offline, host-neutral token and subscription lifecycle.
pub mod lifecycle;

/// Host-neutral executable account, callback and webhook application boundary.
pub mod application;
pub mod coordinator;
pub mod diagnostics;
pub mod runtime;

pub mod presence {
    use crate::{
        config::Identity,
        provider::{Direction, Event},
    };
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeSet;
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Presence {
        Unknown,
        InBed,
        OutOfBed,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Health {
        Unconfirmed,
        Confirmed,
        Uncertain,
    }
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct State {
        pub identity: Identity,
        pub presence: Presence,
        pub health: Health,
        pub last_observation: Option<i64>,
        pub cursor: Option<i64>,
        pub dedup: BTreeSet<String>,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Outcome {
        pub transition: bool,
        pub action: Option<&'static str>,
        pub uncertain: bool,
    }
    impl State {
        pub fn new(identity: Identity) -> Self {
            Self {
                identity,
                presence: Presence::Unknown,
                health: Health::Unconfirmed,
                last_observation: None,
                cursor: None,
                dedup: BTreeSet::new(),
            }
        }
        pub fn restore(s: &str) -> Result<Self, &'static str> {
            if s.len() > 65536 {
                return Err("invalid state");
            }
            let x: Self = serde_json::from_str(s).map_err(|_| "invalid state")?;
            Identity::new(x.identity.user_id.clone(), x.identity.device_id.clone())
                .map_err(|_| "invalid state")?;
            if x.dedup.len() > 256
                || x.dedup.iter().any(|v| v.len() > 256)
                || x.last_observation.is_some_and(|v| v <= 0)
                || x.cursor.is_some_and(|v| v <= 0)
                || (x.presence == Presence::Unknown && x.health == Health::Confirmed)
                || (x.cursor != x.last_observation)
                || (x.presence == Presence::Unknown && x.last_observation.is_some())
                || (x.presence != Presence::Unknown && x.last_observation.is_none())
            {
                return Err("invalid state");
            }
            let mut x = x;
            if x.health == Health::Confirmed {
                x.health = Health::Unconfirmed;
            }
            Ok(x)
        }
        pub fn restore_for(s: &str, expected: &Identity) -> Result<Self, &'static str> {
            let x = Self::restore(s)?;
            if &x.identity != expected {
                return Err("invalid state");
            }
            Ok(x)
        }
        pub fn persist(&self) -> String {
            serde_json::to_string(self).expect("state serialization")
        }
        pub fn apply(&mut self, e: &Event) -> Outcome {
            if e.user_id.is_empty()
                || e.device_id.is_empty()
                || e.user_id.len() > 128
                || e.device_id.len() > 128
                || e.timestamp <= 0
                || (e.direction != Direction::In && e.direction != Direction::Out)
                || e.user_id != self.identity.user_id
                || e.device_id != self.identity.device_id
            {
                return Self::none(false);
            }
            let key = format!(
                "{}:{}:{}",
                e.timestamp,
                if e.direction == Direction::In {
                    "in"
                } else {
                    "out"
                },
                e.device_id
            );
            if self.dedup.contains(&key) || self.cursor.is_some_and(|x| e.timestamp < x) {
                return Self::none(false);
            }
            let next = if e.direction == Direction::In {
                Presence::InBed
            } else {
                Presence::OutOfBed
            };
            if self.cursor == Some(e.timestamp)
                && self.last_observation.is_some()
                && self.presence != next
            {
                self.health = Health::Uncertain;
                return Self::none(true);
            }
            let transition = self.presence != next;
            self.presence = next;
            self.health = Health::Confirmed;
            self.last_observation = Some(e.timestamp);
            self.cursor = Some(e.timestamp);
            self.dedup.insert(key);
            while self.dedup.len() > 256 {
                if let Some(k) = self.dedup.iter().next().cloned() {
                    self.dedup.remove(&k);
                }
            }
            Outcome {
                transition,
                action: if transition {
                    Some(if next == Presence::InBed {
                        "bed_in"
                    } else {
                        "bed_out"
                    })
                } else {
                    None
                },
                uncertain: false,
            }
        }
        fn none(uncertain: bool) -> Outcome {
            Outcome {
                transition: false,
                action: None,
                uncertain,
            }
        }
    }
}

/// Safe host-neutral presentation model. Connection and occupancy are independent.
pub mod presentation {
    use crate::presence::{Health, Presence, State};
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Connection {
        Connected,
        Disconnected,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Occupancy {
        Unknown,
        InBed,
        OutOfBed,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Model {
        pub connection: Connection,
        pub occupancy: Occupancy,
        pub confirmed: bool,
        pub last_event_at: Option<i64>,
    }
    impl Model {
        pub fn from_state(s: &State, connection: Connection) -> Self {
            let occupancy = match s.presence {
                Presence::Unknown => Occupancy::Unknown,
                Presence::InBed => Occupancy::InBed,
                Presence::OutOfBed => Occupancy::OutOfBed,
            };
            Self {
                connection,
                occupancy,
                confirmed: s.health == Health::Confirmed,
                last_event_at: s.last_observation,
            }
        }
        pub fn lines(&self) -> Vec<String> {
            let mut v = vec![
                match self.connection {
                    Connection::Connected => "Connection: connected",
                    Connection::Disconnected => "Connection: disconnected",
                }
                .to_string(),
            ];
            if self.connection == Connection::Disconnected {
                v.push("Occupancy: unavailable while disconnected".into());
            } else {
                v.push(
                    match (self.occupancy, self.confirmed) {
                        (Occupancy::Unknown, _) => "Occupancy: unknown",
                        (Occupancy::InBed, true) => "Occupancy: in bed",
                        (Occupancy::OutOfBed, true) => "Occupancy: out of bed",
                        (Occupancy::InBed, false) => "Occupancy: in bed (unconfirmed)",
                        (Occupancy::OutOfBed, false) => "Occupancy: out of bed (unconfirmed)",
                    }
                    .into(),
                );
                v.push(
                    self.last_event_at
                        .map_or_else(|| "Last event: none".into(), |t| format!("Last event: {t}")),
                );
            }
            v
        }
    }
}

/// Host-neutral persistence-before-publication delivery adapter.
pub mod adapter {
    use crate::{
        config::Identity,
        presence::{Health, Presence, State},
        provider,
    };
    use std::fmt;
    /// A committed event can lose its action if the sink fails, the process crashes, or reading publication fails first. A duplicate event will not retry that action. This is at-most-once attempt behavior, not a reliable outbox or exactly-once guarantee.
    pub trait Store {
        type Error: fmt::Display;
        fn load(&mut self) -> Result<Option<String>, Self::Error>;
        fn save(&mut self, state: &str) -> Result<(), Self::Error>;
    }
    pub trait Sink {
        type Error: fmt::Display;
        fn publish_reading(&mut self, reading: Reading) -> Result<(), Self::Error>;
        fn publish_action(&mut self, action: &'static str) -> Result<(), Self::Error>;
        /// Best-effort observation of the actual delivery path, never a decision input.
        fn observe(&mut self, _event: &provider::Event, _observation: Observation) {}
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Observation {
        Transition,
        Unchanged,
        DuplicateOrStale,
        Conflict,
        StorageFailed,
        ReadingFailed,
        ActionFailed,
        ActionPublished,
        NoAction,
    }
    pub fn host_action(action: &'static str) -> Option<&'static str> {
        match action {
            "bed_in" => Some("bed_in"),
            "bed_out" => Some("bed_out"),
            _ => None,
        }
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Reading {
        Unknown { last_observation: Option<i64> },
        Occupied { occupied: bool, observed_at: i64 },
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct MappedEvent {
        pub reading: Reading,
        pub action: Option<&'static str>,
    }
    pub fn map_state(state: &State, action: Option<&'static str>) -> MappedEvent {
        let reading = match (state.presence, state.health) {
            (Presence::InBed, Health::Confirmed) => Reading::Occupied {
                occupied: true,
                observed_at: state.last_observation.expect("known timestamp"),
            },
            (Presence::OutOfBed, Health::Confirmed) => Reading::Occupied {
                occupied: false,
                observed_at: state.last_observation.expect("known timestamp"),
            },
            _ => Reading::Unknown {
                last_observation: state.last_observation,
            },
        };
        MappedEvent {
            reading,
            action: action.and_then(host_action),
        }
    }
    #[derive(Debug, PartialEq, Eq)]
    pub enum DeliveryError {
        Parse(provider::ParseError),
        Storage,
        ReadingSink,
        ActionSink { action: &'static str },
        InvalidState(&'static str),
    }
    impl fmt::Display for DeliveryError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Parse(error) => write!(f, "{error}"),
                Self::Storage => f.write_str("Storage"),
                Self::ReadingSink => f.write_str("ReadingSink"),
                Self::ActionSink { action } => write!(f, "ActionSink {{ action: \"{action}\" }}"),
                Self::InvalidState(reason) => write!(f, "InvalidState({reason})"),
            }
        }
    }
    impl std::error::Error for DeliveryError {}
    pub struct Adapter<S, H> {
        identity: Identity,
        state: State,
        store: S,
        sink: H,
    }
    impl<S: Store, H: Sink> Adapter<S, H> {
        pub fn new(identity: Identity, mut store: S, sink: H) -> Result<Self, DeliveryError> {
            let state = store
                .load()
                .map_err(|_| DeliveryError::Storage)?
                .map(|s| State::restore_for(&s, &identity).map_err(DeliveryError::InvalidState))
                .transpose()?
                .unwrap_or_else(|| State::new(identity.clone()));
            Ok(Self {
                identity,
                state,
                store,
                sink,
            })
        }
        pub fn state(&self) -> &State {
            &self.state
        }
        pub fn identity(&self) -> &Identity {
            &self.identity
        }
        pub fn deliver(
            &mut self,
            body: &[u8],
            now: i64,
            max_age: i64,
            future_skew: i64,
        ) -> Result<(), DeliveryError> {
            let event = provider::parse(body, &self.identity, now, max_age, future_skew)
                .map_err(DeliveryError::Parse)?;
            let mut candidate = self.state.clone();
            let outcome = candidate.apply(&event);
            self.sink.observe(
                &event,
                if outcome.uncertain {
                    Observation::Conflict
                } else if outcome.action.is_some() {
                    Observation::Transition
                } else if candidate != self.state {
                    Observation::Unchanged
                } else {
                    Observation::DuplicateOrStale
                },
            );
            if candidate != self.state {
                if self.store.save(&candidate.persist()).is_err() {
                    self.sink.observe(&event, Observation::StorageFailed);
                    return Err(DeliveryError::Storage);
                }
                self.state = candidate;
            }
            let mapped = map_state(&self.state, outcome.action);
            if self.sink.publish_reading(mapped.reading).is_err() {
                self.sink.observe(&event, Observation::ReadingFailed);
                return Err(DeliveryError::ReadingSink);
            }
            if let Some(action) = mapped.action {
                if self.sink.publish_action(action).is_err() {
                    self.sink.observe(&event, Observation::ActionFailed);
                    return Err(DeliveryError::ActionSink { action });
                }
                self.sink.observe(&event, Observation::ActionPublished);
            } else {
                self.sink.observe(&event, Observation::NoAction);
            }
            Ok(())
        }
    }
}
