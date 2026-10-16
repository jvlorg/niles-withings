use niles_withings::{
    application::{
        AccountSecretStore, ApplicationError, CallbackResult, SecureAccountRecord, on_callback,
    },
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
    fn set_account_record(&mut self, value: &str) -> Result<(), ()> {
        self.writes += 1;
        if self.writes == self.fail_at {
            return Err(());
        }
        self.bytes = value.into();
        Ok(())
    }
}
struct Provider {
    calls: usize,
    user: u64,
    transport_failure: bool,
}
impl HttpTransport for Provider {
    type Error = ();
    fn post(&mut self, _: HttpRequest) -> Result<HttpResponse, ()> {
        self.calls += 1;
        if self.transport_failure {
            return Err(());
        }
        let (access, refresh) = if self.calls == 1 {
            ("new-access", "new-refresh")
        } else {
            ("access", "")
        };
        if refresh.is_empty() {
            return Err(());
        }
        Ok(HttpResponse { status: 200, body: format!(r#"{{"status":0,"body":{{"userid":{},"access_token":"{}","refresh_token":"{}","expires_in":3600,"scope":"user.sleepevents","token_type":"Bearer"}}}}"#, self.user, access, refresh).into_bytes() })
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
fn callback(d: &mut Disk, p: &mut Provider) -> Result<CallbackResult, ApplicationError> {
    on_callback(
        p,
        d,
        &CallbackUrl::parse("https://example.com/callback").unwrap(),
        &[("code".into(), "synthetic-code".into())],
        None,
        "client",
        &Secret::new("secret").unwrap(),
        4500,
    )
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
fn pending(d: &mut Disk) {
    d.replace_tokens_and_metadata(&tokens(), metadata())
        .unwrap();
    d.begin_refresh(&tokens(), metadata()).unwrap();
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
        transport_failure: false,
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
        transport_failure: false,
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
        transport_failure: false,
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
        transport_failure: false,
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
#[test]
fn restart_pending_then_actual_callback_replaces_and_reconciles() {
    let mut disk = Disk::default();
    pending(&mut disk);
    assert_eq!(
        SecureAccountRecord::restore(&disk.bytes, 123, 4500).unwrap_err(),
        ApplicationError::RecoveryRequired
    );
    let mut callback_provider = Provider {
        calls: 0,
        user: 123,
        transport_failure: false,
    };
    assert_eq!(
        callback(&mut disk, &mut callback_provider),
        Ok(CallbackResult::Stored { user_id: 123 })
    );
    let (restored, restored_meta) = SecureAccountRecord::restore(&disk.bytes, 123, 4500).unwrap();
    assert_eq!(restored.access_token.expose_secret(), "new-access");
    let mut lifecycle = Lifecycle::new(123, Some(restored), Some(restored_meta));
    struct Subscriptions;
    impl HttpTransport for Subscriptions {
        type Error = ();
        fn post(&mut self, _: HttpRequest) -> Result<HttpResponse, ()> {
            Ok(HttpResponse { status: 200, body: br#"{"status":0,"body":{"profiles":[{"appli":50,"callbackurl":"https://example.com/hook","comment":"owned"},{"appli":51,"callbackurl":"https://example.com/hook","comment":"owned"}]}}"#.to_vec() })
        }
    }
    assert_eq!(
        lifecycle
            .reconcile(
                &mut Subscriptions,
                &mut disk,
                "client",
                &Secret::new("secret").unwrap(),
                &CallbackUrl::parse("https://example.com/hook").unwrap(),
                "owned",
                4500
            )
            .status,
        Status::Ready
    );
    assert!(SecureAccountRecord::restore(&disk.bytes, 123, 4500).is_ok());
}

#[test]
fn failed_callback_persistence_preserves_pending_record_and_refuses_restore() {
    let mut disk = Disk::default();
    pending(&mut disk);
    let before = disk.bytes.clone();
    disk.fail_at = disk.writes + 1;
    let mut p = Provider {
        calls: 0,
        user: 123,
        transport_failure: false,
    };
    assert_eq!(callback(&mut disk, &mut p), Err(ApplicationError::Storage));
    assert_eq!(disk.bytes, before);
    assert_eq!(
        SecureAccountRecord::restore(&disk.bytes, 123, 4500).unwrap_err(),
        ApplicationError::RecoveryRequired
    );
}

#[test]
fn ambiguous_transport_failure_latches_runtime_and_persisted_restore() {
    let mut disk = Disk::default();
    let mut lifecycle = Lifecycle::new(123, Some(tokens()), Some(metadata()));
    let mut p = Provider {
        calls: 0,
        user: 123,
        transport_failure: true,
    };
    let first = run(&mut lifecycle, &mut disk, &mut p);
    assert_eq!(first, Status::AuthorizationRequired);
    assert_eq!(run(&mut lifecycle, &mut disk, &mut p), first);
    assert_eq!(p.calls, 1);
    assert_eq!(
        SecureAccountRecord::restore(&disk.bytes, 123, 4500).unwrap_err(),
        ApplicationError::RecoveryRequired
    );
    let restarted_store = Disk {
        bytes: disk.bytes.clone(),
        ..Disk::default()
    };
    assert_eq!(
        SecureAccountRecord::restore(&restarted_store.bytes, 123, 4500).unwrap_err(),
        ApplicationError::RecoveryRequired
    );
    // Restore refused to supply tokens: no lifecycle may be rebuilt from this record.
}

#[test]
fn malformed_unknown_version_oversized_pending_and_debug_are_fail_closed() {
    let mut disk = Disk::default();
    pending(&mut disk);
    let mut wire: serde_json::Value = serde_json::from_str(&disk.bytes).unwrap();
    assert_eq!(wire["version"], 2);
    wire["refresh_pending"] = false.into();
    assert_eq!(
        SecureAccountRecord::restore(&wire.to_string(), 123, 4500),
        Err(ApplicationError::InvalidRecord)
    );
    wire["version"] = 1.into();
    wire["refresh_pending"] = true.into();
    assert_eq!(
        SecureAccountRecord::restore(&wire.to_string(), 123, 4500),
        Err(ApplicationError::InvalidRecord)
    );
    assert!(SecureAccountRecord::restore("not json", 123, 4500).is_err());
    let mut unknown: serde_json::Value = serde_json::from_str(&disk.bytes).unwrap();
    unknown["version"] = serde_json::json!(9);
    assert!(SecureAccountRecord::restore(&unknown.to_string(), 123, 4500).is_err());
    unknown["user_id"] = serde_json::json!(999);
    assert_eq!(
        SecureAccountRecord::restore(&unknown.to_string(), 123, 4500),
        Err(ApplicationError::InvalidRecord)
    );
    let mut foreign_pending: serde_json::Value = serde_json::from_str(&disk.bytes).unwrap();
    foreign_pending["user_id"] = serde_json::json!(999);
    assert_eq!(
        SecureAccountRecord::restore(&foreign_pending.to_string(), 123, 4500),
        Err(ApplicationError::InvalidRecord)
    );
    assert_eq!(
        SecureAccountRecord::restore(&"x".repeat(4097), 123, 4500),
        Err(ApplicationError::InvalidRecord)
    );
    let debug = format!("{:?}", SecureAccountRecord::restore(&disk.bytes, 123, 4500));
    assert!(!debug.contains("old-access") && !debug.contains("old-refresh"));
    disk.replace_tokens_and_metadata(&tokens(), metadata())
        .unwrap();
    let legacy = disk.bytes.replace(",\"refresh_pending\":false", "");
    assert!(SecureAccountRecord::restore(&legacy, 123, 4500).is_ok());
}
