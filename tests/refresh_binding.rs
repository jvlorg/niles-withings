use niles_withings::withings_client::*;
struct Transport;
impl HttpTransport for Transport {
    type Error = ();
    fn post(&mut self, _: HttpRequest) -> Result<HttpResponse, ()> {
        Ok(HttpResponse{status:200,body:br#"{"status":0,"body":{"userid":42,"access_token":"synthetic-access","refresh_token":"synthetic-refresh","expires_in":3600}}"#.to_vec()})
    }
}
#[derive(Default)]
struct Store {
    calls: usize,
    fail: bool,
}
impl SecretStore for Store {
    type Error = ();
    fn replace_tokens(&mut self, _: &TokenSet) -> Result<(), ()> {
        self.calls += 1;
        if self.fail { Err(()) } else { Ok(()) }
    }
}
#[test]
fn foreign_user_never_reaches_store() {
    let mut s = Store::default();
    assert_eq!(
        refresh_bound(
            &mut Transport,
            &mut s,
            "client",
            &Secret::new("secret").unwrap(),
            &Secret::new("refresh").unwrap(),
            43
        ),
        Err(ClientError::AccountMismatch)
    );
    assert_eq!(s.calls, 0);
}
#[test]
fn matching_user_stored_and_storage_failure_preserved() {
    let mut s = Store::default();
    assert!(
        refresh_bound(
            &mut Transport,
            &mut s,
            "client",
            &Secret::new("secret").unwrap(),
            &Secret::new("refresh").unwrap(),
            42
        )
        .is_ok()
    );
    assert_eq!(s.calls, 1);
    s.fail = true;
    assert_eq!(
        refresh_bound(
            &mut Transport,
            &mut s,
            "client",
            &Secret::new("secret").unwrap(),
            &Secret::new("refresh").unwrap(),
            42
        ),
        Err(ClientError::Storage)
    );
}
