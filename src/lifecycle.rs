//! Offline connection lifecycle over the existing provider transport and client.
//!
//! Persist acquisition and expiry timestamps with tokens in one encrypted-store
//! transaction. Refresh begins 300 seconds before expiry; timestamps are Unix
//! seconds and must be positive. A process restart never treats unknown metadata
//! as fresh. Storage failure after refresh is latched because rotation may have
//! invalidated the old refresh token.
use crate::withings_client::{
    self as client, CallbackUrl, ClientError, HttpTransport, Secret, TokenSet,
};
use std::fmt;

const MAX_TOKEN_LIFETIME: u64 = 31 * 24 * 60 * 60;
const REFRESH_LEAD: i64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenMetadata {
    pub acquired_at: i64,
    pub expires_at: i64,
}
impl TokenMetadata {
    fn new(now: i64, lifetime: u64) -> Result<Self, LifecycleError> {
        if now <= 0 || lifetime == 0 || lifetime > MAX_TOKEN_LIFETIME {
            return Err(LifecycleError::InvalidTime);
        }
        let expires_at = now
            .checked_add(i64::try_from(lifetime).map_err(|_| LifecycleError::InvalidTime)?)
            .ok_or(LifecycleError::InvalidTime)?;
        Ok(Self {
            acquired_at: now,
            expires_at,
        })
    }
    fn valid_at(self, now: i64) -> bool {
        now > 0
            && self.acquired_at > 0
            && self.expires_at > self.acquired_at
            && now >= self.acquired_at
            && self
                .expires_at
                .checked_sub(self.acquired_at)
                .is_some_and(|n| n <= MAX_TOKEN_LIFETIME as i64)
            && now < self.expires_at
    }
}

/// Implement with encrypted durable storage and atomic token+metadata replacement.
pub trait SecureTokenStore {
    type Error;
    fn replace_tokens_and_metadata(
        &mut self,
        tokens: &TokenSet,
        metadata: TokenMetadata,
    ) -> Result<(), Self::Error>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    AuthorizationRequired,
    RefreshPersistenceFailed,
    SubscriptionsIncomplete,
    Ready,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    MissingTokens,
    AccountMismatch,
    UnknownOrInvalidMetadata,
    Expired,
    RefreshFailed,
    SubscriptionReadFailed,
    ConflictingOwnedTuple,
    SubscribeFailed,
    ReadbackIncomplete,
    StorageFailed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleStatus {
    pub status: Status,
    pub reason: Option<BlockReason>,
    pub subscriptions: [bool; 2],
}
impl fmt::Display for LifecycleStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Withings lifecycle: {:?}", self.status)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleError {
    InvalidTime,
    AccountMismatch,
    Client(ClientError),
}

/// Injected Unix-seconds clock; callers choose the clock source explicitly.
pub trait Clock {
    fn now(&self) -> i64;
}
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub i64);
impl Clock for FixedClock {
    fn now(&self) -> i64 {
        self.0
    }
}

pub struct Lifecycle {
    expected_user: u64,
    tokens: Option<TokenSet>,
    metadata: Option<TokenMetadata>,
    unusable: Option<BlockReason>,
    last_verified: Option<([bool; 2], TokenMetadata)>,
}
impl Lifecycle {
    /// Missing or invalid persisted metadata fails closed on restart.
    pub fn new(
        expected_user: u64,
        tokens: Option<TokenSet>,
        metadata: Option<TokenMetadata>,
    ) -> Self {
        Self {
            expected_user,
            tokens,
            metadata,
            unusable: None,
            last_verified: None,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile_at<T: HttpTransport, S: SecureTokenStore, C: Clock>(
        &mut self,
        transport: &mut T,
        store: &mut S,
        clock: &C,
        client_id: &str,
        client_secret: &Secret,
        callback: &CallbackUrl,
        comment: &str,
    ) -> LifecycleStatus {
        self.reconcile(
            transport,
            store,
            client_id,
            client_secret,
            callback,
            comment,
            clock.now(),
        )
    }
    /// Current status requires a caller-supplied timestamp; last reconciliation
    /// is only reported Ready while both verified subscription state and tokens remain valid.
    pub fn status(&self, now: i64) -> LifecycleStatus {
        let reason = self.unusable.or_else(|| {
            if self.expected_user == 0 || self.tokens.is_none() {
                Some(BlockReason::MissingTokens)
            } else if self
                .tokens
                .as_ref()
                .is_some_and(|t| t.user_id != self.expected_user)
            {
                Some(BlockReason::AccountMismatch)
            } else if self
                .metadata
                .is_none_or(|m| m.acquired_at <= 0 || m.expires_at <= m.acquired_at)
            {
                Some(BlockReason::UnknownOrInvalidMetadata)
            } else {
                None
            }
        });
        let status = if reason == Some(BlockReason::StorageFailed) {
            Status::RefreshPersistenceFailed
        } else if reason.is_none()
            && self.last_verified.is_some_and(|(subscriptions, metadata)| {
                metadata.valid_at(now) && subscriptions == [true, true]
            })
        {
            Status::Ready
        } else if reason.is_none() {
            Status::SubscriptionsIncomplete
        } else {
            Status::AuthorizationRequired
        };
        LifecycleStatus {
            status,
            reason: if status == Status::Ready {
                None
            } else {
                reason.or(Some(BlockReason::ReadbackIncomplete))
            },
            subscriptions: self.last_verified.map_or([false; 2], |x| x.0),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile<T: HttpTransport, S: SecureTokenStore>(
        &mut self,
        transport: &mut T,
        store: &mut S,
        client_id: &str,
        client_secret: &Secret,
        callback: &CallbackUrl,
        comment: &str,
        now: i64,
    ) -> LifecycleStatus {
        // A failed reconciliation must not retain an earlier Ready snapshot.
        self.last_verified = None;
        if let Some(reason) = self.unusable {
            return self.report(
                if reason == BlockReason::StorageFailed {
                    Status::RefreshPersistenceFailed
                } else {
                    Status::AuthorizationRequired
                },
                Some(reason),
                [false; 2],
            );
        }
        if self.expected_user == 0
            || self
                .tokens
                .as_ref()
                .is_none_or(|t| t.user_id != self.expected_user)
        {
            return self.report(
                Status::AuthorizationRequired,
                Some(BlockReason::AccountMismatch),
                [false; 2],
            );
        }
        if now <= 0 {
            return self.report(
                Status::AuthorizationRequired,
                Some(BlockReason::UnknownOrInvalidMetadata),
                [false; 2],
            );
        }
        let Some(mut tokens) = self.tokens.clone() else {
            return self.report(
                Status::AuthorizationRequired,
                Some(BlockReason::MissingTokens),
                [false; 2],
            );
        };
        let Some(metadata) = self.metadata else {
            return self.report(
                Status::AuthorizationRequired,
                Some(BlockReason::UnknownOrInvalidMetadata),
                [false; 2],
            );
        };
        let access_expired = metadata.expires_at <= now
            && metadata.acquired_at > 0
            && metadata.expires_at > metadata.acquired_at
            && metadata.expires_at - metadata.acquired_at <= MAX_TOKEN_LIFETIME as i64
            && now >= metadata.acquired_at;
        if !metadata.valid_at(now) && !access_expired {
            return self.report(
                Status::AuthorizationRequired,
                Some(BlockReason::Expired),
                [false; 2],
            );
        }
        if metadata.expires_at.saturating_sub(now) <= REFRESH_LEAD {
            tokens = match client::refresh_bound(
                transport,
                &mut AtomicStore { inner: store, now },
                client_id,
                client_secret,
                &tokens.refresh_token,
                self.expected_user,
            ) {
                Ok(t) => t,
                Err(ClientError::Storage) => {
                    self.unusable = Some(BlockReason::StorageFailed);
                    return self.report(
                        Status::RefreshPersistenceFailed,
                        self.unusable,
                        [false; 2],
                    );
                }
                Err(ClientError::AccountMismatch) => {
                    self.unusable = Some(BlockReason::AccountMismatch);
                    return self.report(Status::AuthorizationRequired, self.unusable, [false; 2]);
                }
                Err(_) => {
                    return self.report(
                        Status::AuthorizationRequired,
                        Some(BlockReason::RefreshFailed),
                        [false; 2],
                    );
                }
            };
            self.metadata = match TokenMetadata::new(now, tokens.expires_in) {
                Ok(m) => Some(m),
                Err(_) => {
                    self.unusable = Some(BlockReason::UnknownOrInvalidMetadata);
                    return self.report(Status::AuthorizationRequired, self.unusable, [false; 2]);
                }
            };
            self.tokens = Some(tokens.clone());
        }
        let listed = match client::list_subscriptions(transport, &tokens.access_token) {
            Ok(x) => x,
            Err(_) => {
                return self.report(
                    Status::SubscriptionsIncomplete,
                    Some(BlockReason::SubscriptionReadFailed),
                    [false; 2],
                );
            }
        };
        let mut have = [false; 2];
        for row in &listed {
            if row.callback_url == callback.as_str() && (row.appli == 50 || row.appli == 51) {
                if row.comment != comment {
                    return self.report(
                        Status::SubscriptionsIncomplete,
                        Some(BlockReason::ConflictingOwnedTuple),
                        have,
                    );
                }
                have[usize::from(row.appli == 51)] = true;
            }
        }
        for (i, appli) in [50_u8, 51].into_iter().enumerate() {
            if !have[i]
                && client::subscribe_category(
                    transport,
                    &tokens.access_token,
                    callback,
                    comment,
                    appli,
                )
                .is_err()
            {
                return self.report(
                    Status::SubscriptionsIncomplete,
                    Some(BlockReason::SubscribeFailed),
                    have,
                );
            }
        }
        let readback = match client::list_subscriptions(transport, &tokens.access_token) {
            Ok(x) => x,
            Err(_) => {
                return self.report(
                    Status::SubscriptionsIncomplete,
                    Some(BlockReason::SubscriptionReadFailed),
                    have,
                );
            }
        };
        let mut verified = [false; 2];
        for row in readback
            .iter()
            .filter(|r| r.callback_url == callback.as_str() && (r.appli == 50 || r.appli == 51))
        {
            if row.comment != comment {
                return self.report(
                    Status::SubscriptionsIncomplete,
                    Some(BlockReason::ConflictingOwnedTuple),
                    verified,
                );
            }
            verified[usize::from(row.appli == 51)] = true;
        }
        if verified != [true, true] {
            return self.report(
                Status::SubscriptionsIncomplete,
                Some(BlockReason::ReadbackIncomplete),
                verified,
            );
        }
        self.last_verified = Some((verified, self.metadata.unwrap_or(metadata)));
        self.report(Status::Ready, None, verified)
    }
    fn report(
        &self,
        status: Status,
        reason: Option<BlockReason>,
        subscriptions: [bool; 2],
    ) -> LifecycleStatus {
        LifecycleStatus {
            status,
            reason,
            subscriptions,
        }
    }
}

struct AtomicStore<'a, S> {
    inner: &'a mut S,
    now: i64,
}
impl<S: SecureTokenStore> client::SecretStore for AtomicStore<'_, S> {
    type Error = ();
    fn replace_tokens(&mut self, tokens: &TokenSet) -> Result<(), ()> {
        let metadata = TokenMetadata::new(self.now, tokens.expires_in).map_err(|_| ())?;
        self.inner
            .replace_tokens_and_metadata(tokens, metadata)
            .map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::withings_client::{HttpRequest, HttpResponse};
    use std::collections::VecDeque;
    #[derive(Default)]
    struct Mock {
        responses: VecDeque<HttpResponse>,
        requests: Vec<HttpRequest>,
    }
    impl HttpTransport for Mock {
        type Error = ();
        fn post(&mut self, r: HttpRequest) -> Result<HttpResponse, ()> {
            self.requests.push(r);
            self.responses.pop_front().ok_or(())
        }
    }
    #[derive(Default)]
    struct Store {
        writes: usize,
        fail: bool,
    }
    impl SecureTokenStore for Store {
        type Error = ();
        fn replace_tokens_and_metadata(
            &mut self,
            _: &TokenSet,
            _: TokenMetadata,
        ) -> Result<(), ()> {
            self.writes += 1;
            if self.fail { Err(()) } else { Ok(()) }
        }
    }
    fn r(s: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: s.as_bytes().to_vec(),
        }
    }
    fn list(a: &str, b: &str) -> [HttpResponse; 2] {
        [
            r(&format!(r#"{{"status":0,"body":{{"profiles":{a}}}}}"#)),
            r(&format!(r#"{{"status":0,"body":{{"profiles":{b}}}}}"#)),
        ]
    }
    fn token() -> TokenSet {
        TokenSet {
            user_id: 123,
            access_token: Secret::new("synthetic-access").unwrap(),
            refresh_token: Secret::new("synthetic-refresh").unwrap(),
            expires_in: 10800,
        }
    }
    fn life(expiry: i64) -> Lifecycle {
        Lifecycle::new(
            123,
            Some(token()),
            Some(TokenMetadata {
                acquired_at: 1000,
                expires_at: expiry,
            }),
        )
    }
    fn cb() -> CallbackUrl {
        CallbackUrl::parse("https://example.org/cb").unwrap()
    }
    fn push_lists(t: &mut Mock, a: &str, b: &str) {
        t.responses.extend(list(a, b));
    }
    fn reconcile(l: &mut Lifecycle, t: &mut Mock, store: &mut Store, now: i64) -> LifecycleStatus {
        l.reconcile(
            t,
            store,
            "id",
            &Secret::new("synthetic-client-secret").unwrap(),
            &cb(),
            "niles-owned",
            now,
        )
    }
    #[test]
    fn token_timing_and_no_unnecessary_refresh_or_writes() {
        assert!(TokenMetadata::new(i64::MAX, 1).is_err());
        assert!(TokenMetadata::new(1, MAX_TOKEN_LIFETIME + 1).is_err());
        let mut l = life(10000);
        let mut t = Mock::default();
        push_lists(
            &mut t,
            r#"[{"appli":50,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
            r#"[{"appli":51,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
        );
        push_lists(
            &mut t,
            r#"[{"appli":50,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
            r#"[{"appli":51,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
        );
        let mut s = Store::default();
        assert_eq!(
            reconcile(&mut l, &mut t, &mut s, 1001).status,
            Status::Ready
        );
        assert_eq!(l.status(1001).status, Status::Ready);
        assert_eq!(l.status(1001).reason, None);
        assert_eq!(l.status(10000).status, Status::SubscriptionsIncomplete);
        assert_eq!(l.status(0).status, Status::SubscriptionsIncomplete);
        assert_eq!(t.requests.len(), 4);
        assert_eq!(s.writes, 0);
        assert_ne!(
            reconcile(&mut l, &mut t, &mut s, 1002).status,
            Status::Ready
        );
        assert_ne!(l.status(1002).status, Status::Ready);
    }
    #[test]
    fn expired_and_unknown_restart_metadata_fail_closed() {
        for mut l in [Lifecycle::new(123, Some(token()), None), life(1000)] {
            let mut t = Mock::default();
            assert_eq!(
                reconcile(&mut l, &mut t, &mut Store::default(), 2000).status,
                Status::AuthorizationRequired
            );
            assert!(t.requests.is_empty());
        }
    }
    #[test]
    fn expired_access_refreshes_before_any_subscription_request() {
        let mut l = Lifecycle::new(
            123,
            Some(token()),
            Some(TokenMetadata {
                acquired_at: 500,
                expires_at: 1000,
            }),
        );
        let mut t = Mock::default();
        t.responses.push_back(r(r#"{"status":0,"body":{"userid":123,"access_token":"fresh-access","refresh_token":"fresh-refresh","expires_in":9000}}"#));
        push_lists(
            &mut t,
            r#"[{"appli":50,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
            r#"[{"appli":51,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
        );
        push_lists(
            &mut t,
            r#"[{"appli":50,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
            r#"[{"appli":51,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
        );
        push_lists(
            &mut t,
            r#"[{"appli":50,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
            r#"[{"appli":51,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#,
        );
        assert_eq!(
            reconcile(&mut l, &mut t, &mut Store::default(), 1100).status,
            Status::Ready
        );
        assert!(t.requests[0].form.contains("grant_type=refresh_token"));
        assert_eq!(
            t.requests[1].bearer.as_ref().unwrap().expose_secret(),
            "fresh-access"
        );
    }
    #[test]
    fn refresh_user_mismatch_never_stores_and_rotation_failure_latches() {
        let mut l = life(1400);
        let mut t = Mock::default();
        t.responses.push_back(r(r#"{"status":0,"body":{"userid":999,"access_token":"a","refresh_token":"b","expires_in":9000}}"#));
        let mut s = Store::default();
        assert_eq!(
            reconcile(&mut l, &mut t, &mut s, 1100).reason,
            Some(BlockReason::AccountMismatch)
        );
        assert_eq!(s.writes, 0);
        let mut l = life(1400);
        let mut t = Mock::default();
        t.responses.push_back(r(r#"{"status":0,"body":{"userid":123,"access_token":"a","refresh_token":"b","expires_in":9000}}"#));
        let mut s = Store {
            fail: true,
            ..Store::default()
        };
        assert_eq!(
            reconcile(&mut l, &mut t, &mut s, 1100).status,
            Status::RefreshPersistenceFailed
        );
        let n = t.requests.len();
        assert_eq!(
            reconcile(&mut l, &mut t, &mut s, 1101).status,
            Status::RefreshPersistenceFailed
        );
        assert_eq!(t.requests.len(), n);
    }
    #[test]
    fn conflicting_tuple_blocks_and_no_writes() {
        let mut l = life(10000);
        let mut t = Mock::default();
        push_lists(
            &mut t,
            r#"[{"appli":50,"callbackurl":"https://example.org/cb","comment":"foreign"}]"#,
            "[]",
        );
        let s = reconcile(&mut l, &mut t, &mut Store::default(), 2000);
        assert_eq!(s.reason, Some(BlockReason::ConflictingOwnedTuple));
        assert_eq!(t.requests.len(), 2);
        assert!(!format!("{s:?}").contains("synthetic-access"));
    }
    #[test]
    fn partial_subscribe_requires_readback_and_retry_starts_with_read() {
        let mut l = life(10000);
        let mut t = Mock::default();
        push_lists(&mut t, "[]", "[]");
        t.responses.push_back(r(r#"{"status":0,"body":{}}"#));
        t.responses.push_back(r(r#"{"status":0,"body":{}}"#));
        push_lists(&mut t, "[]", "[]");
        let s = reconcile(&mut l, &mut t, &mut Store::default(), 2000);
        assert_eq!(s.reason, Some(BlockReason::ReadbackIncomplete));
        assert_ne!(s.status, Status::Ready);
    }
    #[test]
    fn restored_metadata_rejects_future_and_excessive_lifetime() {
        for metadata in [
            TokenMetadata {
                acquired_at: 3000,
                expires_at: 4000,
            },
            TokenMetadata {
                acquired_at: 1,
                expires_at: i64::MAX,
            },
        ] {
            let mut l = Lifecycle::new(123, Some(token()), Some(metadata));
            let mut t = Mock::default();
            assert_ne!(
                reconcile(&mut l, &mut t, &mut Store::default(), 2000).status,
                Status::Ready
            );
            assert!(t.requests.is_empty());
        }
    }
    #[test]
    fn refresh_success_persists_metadata_then_reconciles_only_missing_direction() {
        let mut l = life(1400);
        let mut t = Mock::default();
        t.responses.push_back(r(r#"{"status":0,"body":{"userid":123,"access_token":"new-access","refresh_token":"new-refresh","expires_in":9000}}"#));
        let inside =
            r#"[{"appli":50,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#;
        let outside =
            r#"[{"appli":51,"callbackurl":"https://example.org/cb","comment":"niles-owned"}]"#;
        push_lists(&mut t, inside, "[]");
        t.responses.push_back(r(r#"{"status":0,"body":{}}"#));
        push_lists(&mut t, inside, outside);
        let mut s = Store::default();
        assert_eq!(
            reconcile(&mut l, &mut t, &mut s, 1100).status,
            Status::Ready
        );
        assert_eq!(s.writes, 1);
        assert_eq!(
            l.metadata,
            Some(TokenMetadata {
                acquired_at: 1100,
                expires_at: 10100
            })
        );
        let writes: Vec<_> = t
            .requests
            .iter()
            .filter(|x| x.form.contains("action=subscribe"))
            .collect();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].form.contains("appli=51"));
    }
    #[test]
    fn partial_failure_never_ready() {
        let mut l = life(10000);
        let mut t = Mock::default();
        push_lists(&mut t, "[]", "[]");
        t.responses.push_back(r(r#"{"status":0,"body":{}}"#));
        t.responses.push_back(r(r#"{"status":5,"body":{}}"#));
        let s = reconcile(&mut l, &mut t, &mut Store::default(), 2000);
        assert_eq!(s.status, Status::SubscriptionsIncomplete);
        assert_eq!(s.reason, Some(BlockReason::SubscribeFailed));
    }
}
