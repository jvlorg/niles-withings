//! Bounded best-effort persistence and malformed-request summaries.
use super::*;
use niles_withings::diagnostics::{self, Record};
#[derive(Default)]
struct Noise {
    window: i64,
    first: i64,
    last: i64,
    count: u32,
}
thread_local! {
    static FAILURES: RefCell<u64> = const { RefCell::new(0) };
    static NOISE: RefCell<Noise> = RefCell::new(Noise::default());
}
pub(super) fn reset() {
    FAILURES.with(|v| *v.borrow_mut() = 0);
    NOISE.with(|v| *v.borrow_mut() = Noise::default());
}
pub(super) fn pending() -> (u64, u32) {
    (
        FAILURES.with(|v| *v.borrow()),
        NOISE.with(|v| v.borrow().count),
    )
}
pub(super) fn failure() {
    FAILURES.with(|v| {
        let n = *v.borrow();
        *v.borrow_mut() = n.saturating_add(1);
    });
}
fn flush_failures() {
    let count = FAILURES.with(|v| *v.borrow());
    if count > 0 && diagnostics::note_gaps(&mut HostStore, count).is_ok() {
        FAILURES.with(|v| *v.borrow_mut() = 0);
    }
}
pub(super) fn correlation() -> u64 {
    flush_failures();
    diagnostics::next_correlation(&mut HostStore).unwrap_or_else(|_| {
        failure();
        0
    })
}
pub(super) fn write(record: Record) -> Result<(), diagnostics::Error> {
    flush_failures();
    let result = diagnostics::append(&mut HostStore, record);
    if result.is_err() {
        failure();
    }
    result
}
/// One full rejected request per minute. Further rejects only update fixed
/// memory; timer/next-window writes one count+first+last summary. No raw input.
pub(super) fn allow_rejected(at: i64) -> bool {
    flush(at, false);
    NOISE.with(|v| {
        let mut v = v.borrow_mut();
        if v.window == 0 || at < v.window || at - v.window >= 60 {
            v.window = at;
            true
        } else {
            if v.count == 0 {
                v.first = at;
            }
            v.last = at;
            v.count = v.count.saturating_add(1);
            false
        }
    })
}
pub(super) fn flush(at: i64, force: bool) {
    if force {
        flush_failures();
    }
    let record = NOISE.with(|v| {
        let v = v.borrow();
        if v.count == 0 || (!force && at >= v.window && at - v.window < 60) {
            return None;
        }
        Some(Record {
            received_at: v.last,
            provider_at: None,
            mat: None,
            event: "webhook".into(),
            stage: "receipt".into(),
            outcome: "suppressed".into(),
            correlation: 0,
            count: v.count,
            first_at: v.first.min(v.last),
            last_at: v.last,
        })
    });
    if let Some(mut record) = record {
        record.correlation = correlation();
        if write(record).is_ok() {
            NOISE.with(|v| v.borrow_mut().count = 0);
        }
    }
}
