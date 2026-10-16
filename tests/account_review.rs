use niles_withings::{
    application::{AccountSecretStore, ApplicationError},
    lifecycle::{SecureTokenStore, TokenMetadata},
    withings_client::{Secret, TokenSet},
};
#[derive(Default)]
struct Store {
    writes: usize,
}
impl AccountSecretStore for Store {
    type Error = ();
    fn set_account_record(&mut self, _: &str) -> Result<(), ()> {
        self.writes += 1;
        Ok(())
    }
}
#[test]
fn oversized_refresh_record_returns_error_without_write_or_panic() {
    let tokens = TokenSet {
        user_id: 123,
        access_token: Secret::new("a".repeat(3000)).unwrap(),
        refresh_token: Secret::new("b".repeat(3000)).unwrap(),
        expires_in: 3600,
    };
    let mut store = Store::default();
    assert_eq!(
        store.replace_tokens_and_metadata(
            &tokens,
            TokenMetadata {
                acquired_at: 1000,
                expires_at: 4600
            }
        ),
        Err(ApplicationError::InvalidRecord)
    );
    assert_eq!(store.writes, 0);
}
