//! Observe canonical delivery, then persist diagnostics AFTER essential state/actions.
//! No second parser/reducer and no inference of publication from HTTP status.
use super::*;
use niles_withings::{
    adapter::Observation,
    diagnostics::Record,
    provider::{Direction, Event},
};

thread_local! {
    static TRACE: RefCell<Vec<Record>> = const { RefCell::new(Vec::new()) };
}
pub(super) fn begin() {
    TRACE.with(|t| t.borrow_mut().clear());
}
pub(super) fn observe(event: &Event, observation: Observation) {
    let (stage, outcome) = match observation {
        Observation::Transition => ("decision", "transition"),
        Observation::Unchanged => ("decision", "unchanged"),
        Observation::DuplicateOrStale => ("decision", "duplicate_or_stale"),
        Observation::Conflict => ("decision", "conflict"),
        Observation::StorageFailed => ("persistence", "failed"),
        Observation::ReadingFailed => ("publication", "reading_failed"),
        Observation::ActionFailed => ("publication", "action_failed"),
        Observation::ActionPublished => ("publication", "action_published"),
        Observation::NoAction => ("publication", "no_action"),
    };
    // Include account scope; never persist either raw identity. Length-prefixed
    // input avoids ambiguous account/device boundaries.
    let scope = format!(
        "{}:{}{}",
        event.user_id.len(),
        event.user_id,
        event.device_id
    );
    let mat = niles_withings::diagnostics::safe_mat_ref(&scope);
    TRACE.with(|t| {
        let mut t = t.borrow_mut();
        // One routed adapter emits exactly a decision plus one terminal result.
        if t.len() < 2 {
            t.push(Record {
                received_at: 1,
                provider_at: Some(event.timestamp),
                mat,
                event: match event.direction {
                    Direction::In => "bed_in",
                    Direction::Out => "bed_out",
                }
                .into(),
                stage: stage.into(),
                outcome: outcome.into(),
                correlation: 0,
                count: 1,
                first_at: 1,
                last_at: 1,
            });
        }
    });
}
pub(super) fn finish(timestamp: i64) {
    let mut records = TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()));
    if records.is_empty() && !journal_io::allow_rejected(timestamp) {
        return;
    }
    let correlation = journal_io::correlation();
    let mut receipt = records.first().cloned().unwrap_or(Record {
        received_at: timestamp,
        provider_at: None,
        mat: None,
        event: "webhook".into(),
        stage: "receipt".into(),
        outcome: "received".into(),
        correlation,
        count: 1,
        first_at: timestamp,
        last_at: timestamp,
    });
    receipt.stage = "receipt".into();
    receipt.outcome = "received".into();
    if records.is_empty() {
        let mut rejected = receipt.clone();
        rejected.stage = "decision".into();
        rejected.outcome = "blocked_or_rejected".into();
        records.push(rejected);
    }
    records.insert(0, receipt);
    for mut record in records {
        record.received_at = timestamp;
        record.first_at = timestamp;
        record.last_at = timestamp;
        record.correlation = correlation;
        let _ = journal_io::write(record);
    }
}
