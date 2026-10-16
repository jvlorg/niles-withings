use niles_withings::{
    application::{AccountSecretStore, ApplicationError, SecureAccountRecord},
    lifecycle::{Lifecycle, SecureTokenStore, Status, TokenMetadata},
    withings_client::{CallbackUrl, HttpRequest, HttpResponse, HttpTransport, Secret, TokenSet},
};
#[derive(Default)]
struct Disk {
    bytes: String,
    writes: usize,
    fail_at: usize,
}
impl AccountSecretStore for Disk {
    type Error = ();
    fn set_account_record(&mut self, s: &str) -> Result<(), ()> {
        self.writes += 1;
        if self.writes == self.fail_at {
            return Err(());
        }
        self.bytes = s.into();
        Ok(())
    }
}
struct Provider {
    calls: usize,
    user: u64,
}
impl HttpTransport for Provider {
    type Error = ();
    fn post(&mut self, _: HttpRequest) -> Result<HttpResponse, ()> {
        self.calls += 1;
        if self.calls > 1 {
            return Err(());
        }
        Ok(HttpResponse{status:200,body:format!(r#"{{"status":0,"body":{{"userid":{},"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600,"scope":"user.sleepevents","token_type":"Bearer"}}}}"#,self.user).into_bytes()})
    }
}
fn tokens() -> TokenSet {
    TokenSet {
        user_id: 123,
        access_token: Secret::new("old-access").unwrap(),
        refresh_token: Secret::new("old-refresh").unwrap(),
        expires_in: 3600,
    }
}
fn metadata() -> TokenMetadata {
    TokenMetadata {
        acquired_at: 1000,
        expires_at: 4600,
    }
}
fn run(l: &mut Lifecycle, d: &mut Disk, p: &mut Provider) -> Status {
    l.reconcile(
        p,
        d,
        "client",
        &Secret::new("secret").unwrap(),
        &CallbackUrl::parse("https://example.com/hook").unwrap(),
        "owned",
        4500,
    )
    .status
}
#[test]
fn pending_restart_and_clean_replacement() {
    let mut d = Disk::default();
    d.replace_tokens_and_metadata(&tokens(), metadata())
        .unwrap();
    assert!(SecureAccountRecord::restore(&d.bytes, 123, 4500).is_ok());
    d.begin_refresh(&tokens(), metadata()).unwrap();
    assert_eq!(
        SecureAccountRecord::restore(&d.bytes, 123, 4500).unwrap_err(),
        ApplicationError::RecoveryRequired
    );
    d.replace_tokens_and_metadata(&tokens(), metadata())
        .unwrap();
    assert!(SecureAccountRecord::restore(&d.bytes, 123, 4500).is_ok());
    let old = d.bytes.replace(",\"refresh_pending\":false", "");
    assert!(SecureAccountRecord::restore(&old, 123, 4500).is_ok());
}
#[test]
fn rotated_but_unsaved_is_blocked_after_restart() {
    let mut d = Disk::default();
    d.replace_tokens_and_metadata(&tokens(), metadata())
        .unwrap();
    d.fail_at = 3;
    let mut l = Lifecycle::new(123, Some(tokens()), Some(metadata()));
    let mut p = Provider {
        calls: 0,
        user: 123,
    };
    assert_eq!(
        run(&mut l, &mut d, &mut p),
        Status::RefreshPersistenceFailed
    );
    assert_eq!(p.calls, 1);
    drop(l);
    assert_eq!(
        SecureAccountRecord::restore(&d.bytes, 123, 4500).unwrap_err(),
        ApplicationError::RecoveryRequired
    );
}
#[test]
fn intent_failure_sends_nothing() {
    let mut d = Disk::default();
    d.replace_tokens_and_metadata(&tokens(), metadata())
        .unwrap();
    let before = d.bytes.clone();
    d.fail_at = 2;
    let mut l = Lifecycle::new(123, Some(tokens()), Some(metadata()));
    let mut p = Provider {
        calls: 0,
        user: 123,
    };
    assert_eq!(
        run(&mut l, &mut d, &mut p),
        Status::RefreshPersistenceFailed
    );
    assert_eq!(p.calls, 0);
    assert_eq!(before, d.bytes);
}
#[test]
fn completed_refresh_restores_new_pair() {
    let mut d = Disk::default();
    let mut l = Lifecycle::new(123, Some(tokens()), Some(metadata()));
    let mut p = Provider {
        calls: 0,
        user: 123,
    };
    assert_eq!(run(&mut l, &mut d, &mut p), Status::SubscriptionsIncomplete);
    let (t, m) = SecureAccountRecord::restore(&d.bytes, 123, 4500).unwrap();
    assert_eq!(t.refresh_token.expose_secret(), "new-refresh");
    assert_eq!(m.acquired_at, 4500);
}
#[test]
fn foreign_response_keeps_pending_and_blocks_retry() {
    let mut d = Disk::default();
    let mut l = Lifecycle::new(123, Some(tokens()), Some(metadata()));
    let mut p = Provider {
        calls: 0,
        user: 999,
    };
    assert_eq!(run(&mut l, &mut d, &mut p), Status::AuthorizationRequired);
    assert_eq!(run(&mut l, &mut d, &mut p), Status::AuthorizationRequired);
    assert_eq!(p.calls, 1);
    assert_eq!(
        SecureAccountRecord::restore(&d.bytes, 123, 4500).unwrap_err(),
        ApplicationError::RecoveryRequired
    );
    assert!(!d.bytes.contains("new-refresh"));
}
