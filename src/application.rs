//! Host-neutral executable boundary for host-validated account callbacks and webhooks.
//! The caller MUST authenticate browser state/person and webhook capability before invoking these APIs.
use crate::{
    adapter::{Adapter, DeliveryError, Sink, Store},
    config::Identity,
    lifecycle::TokenMetadata,
    withings_client::{self, CallbackUrl, ClientError, HttpTransport, Secret, TokenSet},
};
use serde::{Deserialize, Serialize};
use std::fmt;

const RECORD_VERSION: u8 = 1;
const MAX_RECORD_BYTES: usize = 4096;
const MAX_CODE_BYTES: usize = 2048;
const MAX_WEBHOOK_BODY: usize = 4096;

/// One JSON blob: host encrypted-secret `set` must atomically replace its previous value.
/// It is serialized only for the encrypted secret-store seam; never return it to a UI.
#[derive(Clone)]
pub struct SecureAccountRecord {
    version: u8,
    user_id: u64,
    access_token: SecretText,
    refresh_token: SecretText,
    acquired_at: i64,
    expires_at: i64,
    refresh_pending: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct SecretText(String);
#[derive(Serialize, Deserialize)]
struct AccountRecordWire {
    version: u8,
    user_id: u64,
    access_token: SecretText,
    refresh_token: SecretText,
    acquired_at: i64,
    expires_at: i64,
    #[serde(default)]
    refresh_pending: bool,
}
impl fmt::Debug for SecureAccountRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecureAccountRecord")
            .field("version", &self.version)
            .field("user_id", &"[REDACTED]")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("acquired_at", &self.acquired_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}
impl SecureAccountRecord {
    pub fn from_tokens(
        tokens: &TokenSet,
        metadata: TokenMetadata,
    ) -> Result<Self, ApplicationError> {
        if tokens.user_id == 0
            || metadata.acquired_at <= 0
            || metadata.expires_at <= metadata.acquired_at
            || u64::try_from(
                metadata
                    .expires_at
                    .checked_sub(metadata.acquired_at)
                    .unwrap_or(i64::MAX),
            )
            .unwrap_or(u64::MAX)
                > crate::lifecycle::MAX_TOKEN_LIFETIME
        {
            return Err(ApplicationError::InvalidRecord);
        }
        let record = Self {
            version: RECORD_VERSION,
            user_id: tokens.user_id,
            access_token: SecretText(tokens.access_token.expose_secret().to_owned()),
            refresh_token: SecretText(tokens.refresh_token.expose_secret().to_owned()),
            acquired_at: metadata.acquired_at,
            expires_at: metadata.expires_at,
            refresh_pending: false,
        };
        if record.serialize()?.len() > MAX_RECORD_BYTES {
            return Err(ApplicationError::InvalidRecord);
        }
        Ok(record)
    }
    pub fn user_id(&self) -> u64 {
        self.user_id
    }
    pub fn access_token(&self) -> Result<Secret, ApplicationError> {
        let serialized = self.serialize()?;
        let wire: AccountRecordWire =
            serde_json::from_str(&serialized).map_err(|_| ApplicationError::InvalidRecord)?;
        Secret::new(wire.access_token.0).map_err(|_| ApplicationError::InvalidRecord)
    }
    pub fn serialize(&self) -> Result<String, ApplicationError> {
        let text = serde_json::to_string(&AccountRecordWire {
            version: self.version,
            user_id: self.user_id,
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            acquired_at: self.acquired_at,
            expires_at: self.expires_at,
            refresh_pending: self.refresh_pending,
        })
        .map_err(|_| ApplicationError::InvalidRecord)?;
        if text.len() > MAX_RECORD_BYTES {
            return Err(ApplicationError::InvalidRecord);
        }
        Ok(text)
    }
    pub fn restore(
        serialized: &str,
        expected_user: u64,
        now: i64,
    ) -> Result<(TokenSet, TokenMetadata), ApplicationError> {
        if serialized.len() > MAX_RECORD_BYTES || expected_user == 0 || now <= 0 {
            return Err(ApplicationError::InvalidRecord);
        }
        let wire: AccountRecordWire =
            serde_json::from_str(serialized).map_err(|_| ApplicationError::InvalidRecord)?;
        let record = Self {
            version: wire.version,
            user_id: wire.user_id,
            access_token: wire.access_token,
            refresh_token: wire.refresh_token,
            acquired_at: wire.acquired_at,
            expires_at: wire.expires_at,
            refresh_pending: wire.refresh_pending,
        };
        let lifetime = record
            .expires_at
            .checked_sub(record.acquired_at)
            .ok_or(ApplicationError::InvalidRecord)?;
        if !((record.version == RECORD_VERSION && !record.refresh_pending)
            || (record.version == 2 && record.refresh_pending))
            || record.user_id != expected_user
            || record.access_token.0.is_empty()
            || record.access_token.0.len() > 8192
            || record.refresh_token.0.is_empty()
            || record.refresh_token.0.len() > 8192
            || record.acquired_at <= 0
            || record.expires_at <= record.acquired_at
            || lifetime > crate::lifecycle::MAX_TOKEN_LIFETIME as i64
            || now < record.acquired_at
        {
            return Err(ApplicationError::InvalidRecord);
        }
        if record.refresh_pending {
            return Err(ApplicationError::RecoveryRequired);
        }
        let tokens = TokenSet {
            user_id: record.user_id,
            access_token: Secret::new(record.access_token.0)
                .map_err(|_| ApplicationError::InvalidRecord)?,
            refresh_token: Secret::new(record.refresh_token.0)
                .map_err(|_| ApplicationError::InvalidRecord)?,
            expires_in: u64::try_from(lifetime).map_err(|_| ApplicationError::InvalidRecord)?,
        };
        Ok((
            tokens,
            TokenMetadata {
                acquired_at: record.acquired_at,
                expires_at: record.expires_at,
            },
        ))
    }
}

pub trait AccountSecretStore {
    type Error;
    /// Atomically replaces the same encrypted account record written by callback.
    fn set_account_record(&mut self, serialized: &str) -> Result<(), Self::Error>;
}

/// Durable non-secret storage for the selected account/device binding. This is
/// separate from the encrypted account record: hosts need not offer a
/// transaction spanning their secret and ordinary stores.
pub trait SelectionStore {
    type Error;
    fn load_selection(&mut self) -> Result<Option<String>, Self::Error>;
    fn save_selection(&mut self, serialized: &str) -> Result<(), Self::Error>;
}

const SELECTION_VERSION: u8 = 1;
const MAX_SELECTION_BYTES: usize = 512;

/// A bounded, versioned account/device choice. Opaque IDs are redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct Selection {
    version: u8,
    user_id: u64,
    device_id: String,
}
#[derive(Serialize, Deserialize)]
struct SelectionWire {
    version: u8,
    user_id: u64,
    device_id: String,
}
impl fmt::Debug for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Selection")
            .field("version", &self.version)
            .field("user_id", &"[REDACTED]")
            .field("device_id", &"[REDACTED]")
            .finish()
    }
}
impl Selection {
    fn new(user_id: u64, device_id: &str) -> Result<Self, ApplicationError> {
        if user_id == 0
            || device_id.is_empty()
            || device_id.len() > 128
            || device_id.bytes().any(|b| b.is_ascii_control())
        {
            return Err(ApplicationError::InvalidSelection);
        }
        Ok(Self {
            version: SELECTION_VERSION,
            user_id,
            device_id: device_id.into(),
        })
    }
    pub fn serialize(&self) -> Result<String, ApplicationError> {
        let text = serde_json::to_string(&SelectionWire {
            version: self.version,
            user_id: self.user_id,
            device_id: self.device_id.clone(),
        })
        .map_err(|_| ApplicationError::InvalidSelection)?;
        if text.len() > MAX_SELECTION_BYTES {
            return Err(ApplicationError::InvalidSelection);
        }
        Ok(text)
    }
    pub fn restore(serialized: &str) -> Result<Self, ApplicationError> {
        if serialized.len() > MAX_SELECTION_BYTES {
            return Err(ApplicationError::InvalidSelection);
        }
        let wire: SelectionWire =
            serde_json::from_str(serialized).map_err(|_| ApplicationError::InvalidSelection)?;
        let selection = Self::new(wire.user_id, &wire.device_id)?;
        if wire.version != SELECTION_VERSION {
            return Err(ApplicationError::InvalidSelection);
        }
        Ok(selection)
    }
    pub fn binding_for(&self, account: u64) -> Result<Binding, ApplicationError> {
        if self.user_id != account {
            return Err(ApplicationError::InvalidSelection);
        }
        Binding::new(self.user_id, &self.device_id)
    }
}

/// The only setup output used to construct an event adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct Binding(Identity);
impl Binding {
    fn new(user_id: u64, device_id: &str) -> Result<Self, ApplicationError> {
        Ok(Self(
            Identity::new(user_id.to_string(), device_id)
                .map_err(|_| ApplicationError::InvalidSelection)?,
        ))
    }
    pub fn identity(&self) -> Identity {
        self.0.clone()
    }
}
impl fmt::Debug for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Binding([REDACTED])")
    }
}

/// Sanitized host-neutral setup presentation; it contains no token or opaque ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupPresentation {
    AccountRequired,
    NoEligibleDevice,
    CandidateAvailable,
    ChooseDevice { candidates: usize },
    Selected,
    StaleSelection,
}
pub enum SetupResolution {
    Unavailable,
    Candidate {
        account: u64,
        device: withings_client::Device,
    },
    Choose {
        account: u64,
        devices: Vec<withings_client::Device>,
    },
    Selected(Binding),
    StaleSelection,
}
impl SetupResolution {
    pub fn presentation(&self) -> SetupPresentation {
        match self {
            Self::Unavailable => SetupPresentation::NoEligibleDevice,
            Self::Candidate { .. } => SetupPresentation::CandidateAvailable,
            Self::Choose { devices, .. } => SetupPresentation::ChooseDevice {
                candidates: devices.len(),
            },
            Self::Selected(_) => SetupPresentation::Selected,
            Self::StaleSelection => SetupPresentation::StaleSelection,
        }
    }
}
impl fmt::Debug for SetupResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "SetupResolution::Unavailable",
            Self::Candidate { .. } => "SetupResolution::Candidate([REDACTED])",
            Self::Choose { .. } => "SetupResolution::Choose([REDACTED])",
            Self::Selected(_) => "SetupResolution::Selected([REDACTED])",
            Self::StaleSelection => "SetupResolution::StaleSelection",
        })
    }
}
impl<S: AccountSecretStore> crate::lifecycle::SecureTokenStore for S {
    type Error = ApplicationError;
    fn begin_refresh(
        &mut self,
        tokens: &TokenSet,
        metadata: TokenMetadata,
    ) -> Result<(), Self::Error> {
        let mut record = SecureAccountRecord::from_tokens(tokens, metadata)?;
        // Old readers must reject pending records, never reuse the old token.
        record.version = 2;
        record.refresh_pending = true;
        self.set_account_record(&record.serialize()?)
            .map_err(|_| ApplicationError::Storage)
    }
    fn replace_tokens_and_metadata(
        &mut self,
        tokens: &TokenSet,
        metadata: TokenMetadata,
    ) -> Result<(), Self::Error> {
        let record = SecureAccountRecord::from_tokens(tokens, metadata)?;
        let serialized = record.serialize()?;
        self.set_account_record(&serialized)
            .map_err(|_| ApplicationError::Storage)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationError {
    InvalidCallback,
    AccessDenied,
    InvalidRecord,
    RecoveryRequired,
    InvalidSelection,
    Provider,
    Storage,
    InvalidWebhook,
    Delivery,
}
impl fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Withings setup or delivery failed ({self:?})")
    }
}
impl std::error::Error for ApplicationError {}

#[derive(PartialEq, Eq)]
pub enum CallbackResult {
    Stored { user_id: u64 },
    Denied,
}
impl fmt::Debug for CallbackResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stored { .. } => f.write_str("Stored { user_id: [REDACTED] }"),
            Self::Denied => f.write_str("Denied"),
        }
    }
}

/// `query` contains decoded key/value pairs after the host has checked state/person and removed state.
/// This code accepts exactly one bounded `code`, OR exactly one bounded denial `error`.
#[allow(clippy::too_many_arguments)]
pub fn on_callback<T: HttpTransport, S: AccountSecretStore>(
    transport: &mut T,
    store: &mut S,
    callback: &CallbackUrl,
    query: &[(String, String)],
    person: Option<&str>,
    client_id: &str,
    client_secret: &Secret,
    now: i64,
) -> Result<CallbackResult, ApplicationError> {
    if person.is_some_and(|p| p.len() > 256) || query.len() != 1 || now <= 0 {
        return Err(ApplicationError::InvalidCallback);
    }
    let (key, value) = &query[0];
    if value.is_empty() || value.len() > MAX_CODE_BYTES || key.len() > 32 {
        return Err(ApplicationError::InvalidCallback);
    }
    if key == "error" {
        return Ok(CallbackResult::Denied);
    }
    if key != "code" || value.bytes().any(|b| b.is_ascii_control()) {
        return Err(ApplicationError::InvalidCallback);
    }
    let code = Secret::new(value.clone()).map_err(|_| ApplicationError::InvalidCallback)?;
    let tokens = withings_client::exchange_code_unstored(
        transport,
        client_id,
        client_secret,
        &code,
        callback.as_str(),
    )
    .map_err(map_client)?;
    if tokens.user_id == 0 {
        return Err(ApplicationError::InvalidRecord);
    }
    let metadata = TokenMetadata::new_for_account(now, tokens.expires_in)?;
    let record = SecureAccountRecord::from_tokens(&tokens, metadata)?;
    let serialized = record.serialize()?;
    store
        .set_account_record(&serialized)
        .map_err(|_| ApplicationError::Storage)?;
    Ok(CallbackResult::Stored {
        user_id: tokens.user_id,
    })
}
pub fn discover_account_devices<T: HttpTransport>(
    transport: &mut T,
    record: &SecureAccountRecord,
    authenticated_user: u64,
    selected_id: Option<&str>,
    now: i64,
) -> Result<withings_client::DeviceSelection, ApplicationError> {
    if record.user_id() != authenticated_user || authenticated_user == 0 {
        return Ok(withings_client::DeviceSelection::AccountMismatch);
    }
    if now < record.acquired_at || now >= record.expires_at {
        return Err(ApplicationError::InvalidRecord);
    }
    let devices =
        withings_client::list_devices(transport, &record.access_token()?).map_err(map_client)?;
    Ok(withings_client::select_sleep_device(
        record.user_id(),
        authenticated_user,
        &devices,
        selected_id,
    ))
}

/// Load the durable choice and discover the current account's mats. A choice
/// from another OAuth account is unconfigured; a missing or conflicting chosen
/// device remains stale and never auto-switches.
pub fn setup_discover<T: HttpTransport, P: SelectionStore>(
    transport: &mut T,
    selections: &mut P,
    record: &SecureAccountRecord,
    authenticated_user: u64,
    now: i64,
) -> Result<SetupResolution, ApplicationError> {
    if record.user_id() != authenticated_user || authenticated_user == 0 {
        return Err(ApplicationError::InvalidRecord);
    }
    let saved = selections
        .load_selection()
        .map_err(|_| ApplicationError::Storage)?
        .map(|value| Selection::restore(&value))
        .transpose()?;
    let selected_id = saved.as_ref().and_then(|selection| {
        (selection.user_id == authenticated_user).then_some(selection.device_id.as_str())
    });
    match discover_account_devices(transport, record, authenticated_user, selected_id, now)? {
        withings_client::DeviceSelection::Unavailable => Ok(SetupResolution::Unavailable),
        withings_client::DeviceSelection::Candidate(device) => Ok(SetupResolution::Candidate {
            account: authenticated_user,
            device,
        }),
        withings_client::DeviceSelection::Choose(devices) => Ok(SetupResolution::Choose {
            account: authenticated_user,
            devices,
        }),
        withings_client::DeviceSelection::Selected(device) => Ok(SetupResolution::Selected(
            Selection::new(authenticated_user, &device.device_id)?
                .binding_for(authenticated_user)?,
        )),
        withings_client::DeviceSelection::StaleSelection => Ok(SetupResolution::StaleSelection),
        withings_client::DeviceSelection::AccountMismatch => Err(ApplicationError::InvalidRecord),
    }
}

/// Explicitly accept a discovered candidate. Save completes before a binding
/// is returned, so save failure cannot activate webhook processing.
pub fn accept_selection<P: SelectionStore>(
    selections: &mut P,
    authenticated_user: u64,
    discovered: &SetupResolution,
    device_id: &str,
) -> Result<Binding, ApplicationError> {
    let matches = match discovered {
        SetupResolution::Candidate { account, device } => {
            *account == authenticated_user && device.device_id == device_id
        }
        SetupResolution::Choose { account, devices } => {
            *account == authenticated_user
                && devices.iter().any(|device| device.device_id == device_id)
        }
        _ => false,
    };
    if !matches {
        return Err(ApplicationError::InvalidSelection);
    }
    let selection = Selection::new(authenticated_user, device_id)?;
    selections
        .save_selection(&selection.serialize()?)
        .map_err(|_| ApplicationError::Storage)?;
    selection.binding_for(authenticated_user)
}

/// Construct the existing event adapter only from a durable setup binding.
pub fn adapter_from_binding<S: Store, H: Sink>(
    binding: &Binding,
    store: S,
    sink: H,
) -> Result<Adapter<S, H>, DeliveryError> {
    Adapter::new(binding.identity(), store, sink)
}

fn map_client(_: ClientError) -> ApplicationError {
    ApplicationError::Provider
}

#[derive(Clone, PartialEq, Eq)]
pub struct WebhookRequest {
    pub method: String,
    pub content_type: String,
    pub body: Vec<u8>,
}
impl fmt::Debug for WebhookRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookRequest")
            .field("method", &"[REDACTED]")
            .field("content_type", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .finish()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookResponse {
    pub status: u16,
    pub body: Vec<u8>,
}
impl WebhookResponse {
    fn status(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
        }
    }
}
/// Internal handler only: caller MUST have validated the host webhook capability.
/// No authorization or provider-origin authentication occurs here.
pub fn on_webhook<S: Store, H: Sink>(
    adapter: &mut Adapter<S, H>,
    request: &WebhookRequest,
    now: i64,
    max_age: i64,
    future_skew: i64,
) -> WebhookResponse {
    if request.method != "POST" {
        return WebhookResponse::status(405);
    }
    if request.content_type.split(';').next().is_none_or(|t| {
        !t.trim()
            .eq_ignore_ascii_case("application/x-www-form-urlencoded")
    }) {
        return WebhookResponse::status(415);
    }
    if request.body.len() > MAX_WEBHOOK_BODY {
        return WebhookResponse::status(413);
    }
    if now <= 0 || max_age < 0 || future_skew < 0 {
        return WebhookResponse::status(400);
    }
    match adapter.deliver(&request.body, now, max_age, future_skew) {
        Ok(()) => WebhookResponse::status(200),
        Err(DeliveryError::Parse(_)) | Err(DeliveryError::InvalidState(_)) => {
            WebhookResponse::status(400)
        }
        Err(
            DeliveryError::Storage | DeliveryError::ReadingSink | DeliveryError::ActionSink { .. },
        ) => WebhookResponse::status(503),
    }
}

/// Optional helper over configured public_url; host retains final callback URL authority.
pub fn callback_url(public_url: &str, plugin_id: &str) -> Result<CallbackUrl, ApplicationError> {
    let base = url::Url::parse(public_url).map_err(|_| ApplicationError::InvalidCallback)?;
    if base.scheme() != "https"
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || plugin_id.is_empty()
        || !plugin_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(ApplicationError::InvalidCallback);
    }
    let mut path = base.path().trim_end_matches('/').to_owned();
    path.push_str(&format!("/plugins/{plugin_id}/callback"));
    let mut url = base;
    url.set_path(&path);
    CallbackUrl::parse(url.as_str()).map_err(|_| ApplicationError::InvalidCallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Identity;
    use crate::withings_client::{HttpRequest, HttpResponse};
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};
    #[derive(Default)]
    struct Transport(VecDeque<HttpResponse>);
    impl HttpTransport for Transport {
        type Error = ();
        fn post(&mut self, _: HttpRequest) -> Result<HttpResponse, ()> {
            self.0.pop_front().ok_or(())
        }
    }
    #[derive(Default)]
    struct Secrets {
        value: Option<String>,
        fail: bool,
    }
    impl AccountSecretStore for Secrets {
        type Error = ();
        fn set_account_record(&mut self, s: &str) -> Result<(), ()> {
            if self.fail {
                return Err(());
            }
            self.value = Some(s.to_owned());
            Ok(())
        }
    }
    #[derive(Default)]
    struct Data {
        state: Option<String>,
        actions: Vec<&'static str>,
        fail_load: bool,
        fail_save: bool,
        fail_reading: bool,
        fail_action: bool,
    }
    #[derive(Clone, Default)]
    struct Shared(Rc<RefCell<Data>>);
    impl Store for Shared {
        type Error = &'static str;
        fn load(&mut self) -> Result<Option<String>, &'static str> {
            if self.0.borrow().fail_load {
                Err("load failed")
            } else {
                Ok(self.0.borrow().state.clone())
            }
        }
        fn save(&mut self, s: &str) -> Result<(), &'static str> {
            let mut d = self.0.borrow_mut();
            if d.fail_save {
                return Err("failed");
            }
            d.state = Some(s.to_owned());
            Ok(())
        }
    }
    impl Sink for Shared {
        type Error = &'static str;
        fn publish_reading(&mut self, _: crate::adapter::Reading) -> Result<(), &'static str> {
            if self.0.borrow().fail_reading {
                Err("failed")
            } else {
                Ok(())
            }
        }
        fn publish_action(&mut self, a: &'static str) -> Result<(), &'static str> {
            let mut d = self.0.borrow_mut();
            if d.fail_action {
                Err("action failed")
            } else {
                d.actions.push(a);
                Ok(())
            }
        }
    }
    fn response() -> HttpResponse {
        HttpResponse { status:200, body:br#"{"status":0,"body":{"userid":123,"access_token":"synthetic-access","refresh_token":"synthetic-refresh","expires_in":3600}}"#.to_vec() }
    }
    fn callback() -> CallbackUrl {
        CallbackUrl::parse("https://example.org/plugins/withings/callback").unwrap()
    }
    #[test]
    fn callback_writes_one_bound_record_and_restores_redacted() {
        let mut t = Transport(VecDeque::from([response()]));
        let mut s = Secrets::default();
        let result = on_callback(
            &mut t,
            &mut s,
            &callback(),
            &[("code".into(), "synthetic-code".into())],
            Some("person-1"),
            "client",
            &Secret::new("synthetic-secret").unwrap(),
            1000,
        )
        .unwrap();
        assert_eq!(result, CallbackResult::Stored { user_id: 123 });
        let serialized = s.value.clone().unwrap();
        assert!(serialized.len() <= MAX_RECORD_BYTES);
        assert!(
            !format!(
                "{:?}",
                SecureAccountRecord::restore(&serialized, 123, 1001).unwrap()
            )
            .contains("synthetic-access")
        );
        assert!(SecureAccountRecord::restore(&serialized, 999, 1001).is_err());
        let mut bad = serde_json::from_str::<serde_json::Value>(&serialized).unwrap();
        bad["version"] = serde_json::json!(7);
        assert!(SecureAccountRecord::restore(&bad.to_string(), 123, 1001).is_err());
        let oversized = TokenSet {
            access_token: Secret::new("a".repeat(5000)).unwrap(),
            ..TokenSet {
                user_id: 123,
                access_token: Secret::new("a").unwrap(),
                refresh_token: Secret::new("r").unwrap(),
                expires_in: 3600,
            }
        };
        assert!(
            SecureAccountRecord::from_tokens(
                &oversized,
                TokenMetadata::new_for_account(1000, 3600).unwrap()
            )
            .is_err()
        );
        assert!(SecureAccountRecord::restore(&serialized, 123, 999).is_err());
        let request = WebhookRequest {
            method: "POST-SENTINEL".into(),
            content_type: "SECRET-HEADER".into(),
            body: b"payload-sentinel".to_vec(),
        };
        let debug = format!("{:?}", request);
        assert!(
            !debug.contains("POST-SENTINEL")
                && !debug.contains("SECRET-HEADER")
                && !debug.contains("payload-sentinel")
        );
        let expired_restore = AccountRecordWire {
            version: RECORD_VERSION,
            user_id: 123,
            access_token: SecretText("rotated-access".into()),
            refresh_token: SecretText("rotated-refresh".into()),
            acquired_at: 1000,
            expires_at: 2000,
            refresh_pending: false,
        };
        let expired_json = serde_json::to_string(&expired_restore).unwrap();
        let (tokens, metadata) = SecureAccountRecord::restore(&expired_json, 123, 2001).unwrap();
        let mut store = Secrets::default();
        crate::lifecycle::SecureTokenStore::replace_tokens_and_metadata(
            &mut store, &tokens, metadata,
        )
        .unwrap();
        let (restored, restored_metadata) =
            SecureAccountRecord::restore(store.value.as_deref().unwrap(), 123, 2001).unwrap();
        assert_eq!(restored.access_token.expose_secret(), "rotated-access");
        assert_eq!(restored.refresh_token.expose_secret(), "rotated-refresh");
        assert_eq!(restored_metadata, metadata);
        let mut failing = Secrets {
            value: Some("prior".into()),
            fail: true,
        };
        assert!(
            crate::lifecycle::SecureTokenStore::replace_tokens_and_metadata(
                &mut failing,
                &tokens,
                metadata
            )
            .is_err()
        );
        assert_eq!(failing.value.as_deref(), Some("prior"));
    }
    #[test]
    fn composed_callback_subscription_webhook_restart() {
        struct Clock;
        impl crate::lifecycle::Clock for Clock {
            fn now(&self) -> i64 {
                1001
            }
        }
        let mut transport = Transport(VecDeque::from([response()]));
        let mut secrets = Secrets::default();
        let client_secret = Secret::new("synthetic-secret").unwrap();
        let result = on_callback(
            &mut transport,
            &mut secrets,
            &callback(),
            &[("code".into(), "synthetic-code".into())],
            None,
            "client",
            &client_secret,
            1000,
        )
        .unwrap();
        let CallbackResult::Stored { user_id } = result else {
            panic!("not stored")
        };
        let (tokens, metadata) =
            SecureAccountRecord::restore(secrets.value.as_deref().unwrap(), user_id, 1001).unwrap();
        let mut lifecycle = crate::lifecycle::Lifecycle::new(user_id, Some(tokens), Some(metadata));
        for appli in [50, 51, 50, 51] {
            transport.0.push_back(HttpResponse { status: 200, body: serde_json::to_vec(&serde_json::json!({"status":0,"body":{"profiles":[{"appli":appli,"callbackurl":callback().as_str(),"comment":"owned"}]}})).unwrap() });
        }
        assert_eq!(
            lifecycle
                .reconcile_at(
                    &mut transport,
                    &mut secrets,
                    &Clock,
                    "client",
                    &client_secret,
                    &callback(),
                    "owned"
                )
                .status,
            crate::lifecycle::Status::Ready
        );
        assert!(transport.0.is_empty());
        let shared = Shared::default();
        #[derive(Default)]
        struct SelectionMemory(Option<String>);
        impl SelectionStore for SelectionMemory {
            type Error = ();
            fn load_selection(&mut self) -> Result<Option<String>, ()> {
                Ok(self.0.clone())
            }
            fn save_selection(&mut self, value: &str) -> Result<(), ()> {
                self.0 = Some(value.into());
                Ok(())
            }
        }
        let (tokens, metadata) =
            SecureAccountRecord::restore(secrets.value.as_deref().unwrap(), user_id, 1001).unwrap();
        let record = SecureAccountRecord::from_tokens(&tokens, metadata).unwrap();
        let mut selections = SelectionMemory::default();
        let inventory = || {
            HttpResponse { status: 200, body: br#"{"status":0,"body":{"devices":[{"deviceid":"mat","type":"Sleep Monitor","model":"Aura Sensor","model_id":61}]}}"#.to_vec() }
        };
        transport.0.push_back(inventory());
        let candidate =
            setup_discover(&mut transport, &mut selections, &record, user_id, 1001).unwrap();
        let binding = accept_selection(&mut selections, user_id, &candidate, "mat").unwrap();
        let id = binding.identity();
        let mut adapter = adapter_from_binding(&binding, shared.clone(), shared.clone()).unwrap();
        transport.0.push_back(inventory());
        assert!(matches!(
            setup_discover(&mut transport, &mut selections, &record, user_id, 1001).unwrap(),
            SetupResolution::Selected(_)
        ));
        let event = |appli, date| WebhookRequest {
            method: "POST".into(),
            content_type: "application/x-www-form-urlencoded".into(),
            body: format!("userid={user_id}&deviceid=mat&appli={appli}&date={date}").into_bytes(),
        };
        assert_eq!(
            on_webhook(&mut adapter, &event(50, 1001), 1001, 0, 0).status,
            200
        );
        drop(adapter);
        let mut adapter = Adapter::new(id, shared.clone(), shared.clone()).unwrap();
        assert_eq!(
            on_webhook(&mut adapter, &event(50, 1001), 1001, 0, 0).status,
            200
        );
        assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
        assert_eq!(
            on_webhook(&mut adapter, &event(51, 1002), 1002, 0, 0).status,
            200
        );
        assert_eq!(shared.0.borrow().actions, vec!["bed_in", "bed_out"]);
    }
    #[test]
    fn callback_denial_malformed_and_storage_failure_are_closed() {
        let mut t = Transport::default();
        let mut s = Secrets::default();
        assert_eq!(
            on_callback(
                &mut t,
                &mut s,
                &callback(),
                &[("error".into(), "access_denied".into())],
                Some("p"),
                "client",
                &Secret::new("secret").unwrap(),
                1000
            )
            .unwrap(),
            CallbackResult::Denied
        );
        assert!(s.value.is_none());
        assert!(
            on_callback(
                &mut t,
                &mut s,
                &callback(),
                &[
                    ("code".into(), "x".into()),
                    ("state".into(), "secret".into())
                ],
                Some("p"),
                "client",
                &Secret::new("secret").unwrap(),
                1000
            )
            .is_err()
        );
        let mut t = Transport(VecDeque::from([response()]));
        let mut s = Secrets {
            value: Some("previous-record".into()),
            fail: true,
        };
        assert_eq!(
            on_callback(
                &mut t,
                &mut s,
                &callback(),
                &[("code".into(), "synthetic-code".into())],
                None,
                "client",
                &Secret::new("secret").unwrap(),
                1000
            ),
            Err(ApplicationError::Storage)
        );
        assert_eq!(s.value.as_deref(), Some("previous-record"));
    }
    #[test]
    fn webhook_failure_sites_are_separately_injected_and_retry_is_lossy() {
        let request = WebhookRequest {
            method: "POST".into(),
            content_type: "application/x-www-form-urlencoded".into(),
            body: b"userid=12345&deviceid=mat&appli=50&date=1000".to_vec(),
        };
        let shared = Shared::default();
        shared.0.borrow_mut().fail_load = true;
        assert!(
            Adapter::new(
                Identity::new("12345", "mat").unwrap(),
                shared.clone(),
                shared.clone()
            )
            .is_err()
        );
        let shared = Shared::default();
        shared.0.borrow_mut().fail_save = true;
        let mut adapter = Adapter::new(
            Identity::new("12345", "mat").unwrap(),
            shared.clone(),
            shared.clone(),
        )
        .unwrap();
        assert_eq!(on_webhook(&mut adapter, &request, 1000, 0, 0).status, 503);
        shared.0.borrow_mut().fail_save = false;
        assert_eq!(on_webhook(&mut adapter, &request, 1000, 0, 0).status, 200);
        let shared = Shared::default();
        shared.0.borrow_mut().fail_reading = true;
        let mut adapter = Adapter::new(
            Identity::new("12345", "mat").unwrap(),
            shared.clone(),
            shared.clone(),
        )
        .unwrap();
        assert_eq!(on_webhook(&mut adapter, &request, 1000, 0, 0).status, 503);
        shared.0.borrow_mut().fail_reading = false;
        assert_eq!(on_webhook(&mut adapter, &request, 1000, 0, 0).status, 200);
        let shared = Shared::default();
        shared.0.borrow_mut().fail_action = true;
        let mut adapter = Adapter::new(
            Identity::new("12345", "mat").unwrap(),
            shared.clone(),
            shared.clone(),
        )
        .unwrap();
        let transition = WebhookRequest {
            body: b"userid=12345&deviceid=mat&appli=50&date=1000".to_vec(),
            ..request
        };
        assert_eq!(
            on_webhook(&mut adapter, &transition, 1000, 0, 0).status,
            503
        );
        shared.0.borrow_mut().fail_action = false;
        assert_eq!(
            on_webhook(&mut adapter, &transition, 1000, 0, 0).status,
            200
        );
        assert!(
            shared.0.borrow().actions.is_empty(),
            "committed retry is intentionally not replayed"
        );
    }

    #[test]
    fn webhook_methods_validation_dedup_restart_and_failures() {
        let shared = Shared::default();
        let identity = Identity::new("12345", "mat").unwrap();
        let mut adapter = Adapter::new(identity.clone(), shared.clone(), shared.clone()).unwrap();
        let request = |method: &str, body: Vec<u8>| WebhookRequest {
            method: method.into(),
            content_type: "application/x-www-form-urlencoded; charset=utf-8".into(),
            body,
        };
        assert_eq!(
            on_webhook(&mut adapter, &request("GET", vec![]), 1000, 0, 0).status,
            405
        );
        assert_eq!(
            on_webhook(
                &mut adapter,
                &WebhookRequest {
                    method: "POST".into(),
                    content_type: "text/plain".into(),
                    body: b"userid=12345".to_vec()
                },
                1000,
                0,
                0
            )
            .status,
            415
        );
        assert_eq!(
            on_webhook(
                &mut adapter,
                &request(
                    "POST",
                    b"userid=999&deviceid=mat&appli=50&date=1000".to_vec()
                ),
                1000,
                0,
                0
            )
            .status,
            400
        );
        assert_eq!(
            on_webhook(
                &mut adapter,
                &request(
                    "POST",
                    b"userid=12345&deviceid=mat&appli=50&date=1000".to_vec()
                ),
                1000,
                0,
                0
            )
            .status,
            200
        );
        assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
        drop(adapter);
        let mut restarted = Adapter::new(identity, shared.clone(), shared.clone()).unwrap();
        assert_eq!(
            on_webhook(
                &mut restarted,
                &request(
                    "POST",
                    b"userid=12345&deviceid=mat&appli=50&date=1000".to_vec()
                ),
                1000,
                0,
                0
            )
            .status,
            200
        );
        assert_eq!(shared.0.borrow().actions, vec!["bed_in"]);
        assert_eq!(
            on_webhook(
                &mut restarted,
                &request("POST", vec![b'x'; MAX_WEBHOOK_BODY + 1]),
                1000,
                0,
                0
            )
            .status,
            413
        );
        shared.0.borrow_mut().fail_save = true;
        assert_eq!(
            on_webhook(
                &mut restarted,
                &request(
                    "POST",
                    b"userid=12345&deviceid=mat&appli=51&date=1001".to_vec()
                ),
                1001,
                0,
                0
            )
            .status,
            503
        );
    }
}
