//! Bounded health summaries: transitions immediately, unchanged health hourly.
use super::*;

thread_local! {
    static LAST: RefCell<Option<(&'static str, i64)>> = const { RefCell::new(None) };
    static REASON: RefCell<&'static str> = const { RefCell::new("blocked") };
}
pub(super) fn reset() {
    LAST.with(|last| *last.borrow_mut() = None);
    REASON.with(|v| *v.borrow_mut() = "blocked");
}
pub(super) fn reconciled(status: niles_withings::lifecycle::LifecycleStatus) {
    use niles_withings::lifecycle::{BlockReason, Status};
    let reason = match (status.status, status.reason) {
        (Status::Ready, _) => "ready",
        (Status::RefreshPersistenceFailed, _) | (_, Some(BlockReason::StorageFailed)) => {
            "refresh_store_failed"
        }
        (_, Some(BlockReason::RefreshFailed)) => "refresh_failed",
        (Status::AuthorizationRequired, _) => "authorization_failed",
        (Status::SubscriptionsIncomplete, _) => "subscription_failed",
    };
    REASON.with(|v| *v.borrow_mut() = reason);
}
pub(super) fn sample(stage: &'static str) {
    let Ok(timestamp) = now() else { return };
    let outcome = automatic::FLEET.with(|f| {
        let f = f.borrow();
        if f.ready {
            "ready"
        } else if secrets::get("account").is_none() {
            "disconnected"
        } else {
            let reason = REASON.with(|v| *v.borrow());
            if reason != "ready" && reason != "blocked" {
                reason
            } else if account_id().is_err() {
                "authorization_failed"
            } else if f.account.is_none() {
                "blocked"
            } else if !f.inventory_valid {
                "discovery_failed"
            } else {
                "blocked"
            }
        }
    });
    let emit = LAST.with(|last| {
        let last = last.borrow();
        last.is_none_or(|(previous, at)| {
            previous != outcome || timestamp < at || timestamp - at >= 3600
        })
    });
    if emit && journal(timestamp, "lifecycle", stage, outcome).is_ok() {
        LAST.with(|last| *last.borrow_mut() = Some((outcome, timestamp)));
    }
}
