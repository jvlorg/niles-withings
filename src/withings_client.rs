//! Provider-side OAuth and notification contracts. Transport and durable secret
//! storage are injected; nothing in this module performs ambient network I/O.
use serde::Deserialize;
use std::{collections::HashSet, fmt};
use url::Url;

const AUTH_URL: &str = "https://account.withings.com/oauth2_user/authorize2";
const TOKEN_URL: &str = "https://wbsapi.withings.net/v2/oauth2";
const NOTIFY_URL: &str = "https://wbsapi.withings.net/notify";
const USER_URL: &str = "https://wbsapi.withings.net/v2/user";
pub const BED_SCOPE: &str = "user.sleepevents";
pub const BED_APPLI: u8 = 50;
pub const BED_APPLI_OUT: u8 = 51;
pub const CALLBACK_MAX_BYTES: usize = 255;
const MAX_RESPONSE: usize = 64 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);
impl Secret {
    pub fn new(s: impl Into<String>) -> Result<Self, ClientError> {
        let s = s.into();
        if s.is_empty() || s.len() > 8192 {
            return Err(ClientError::InvalidInput);
        }
        Ok(Self(s))
    }
    /// Explicit secret access for approved transports/encrypted stores only; never log or plaintext-persist it.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientError {
    InvalidInput,
    InvalidCallback,
    StateMismatch,
    StateExpired,
    StateReplayed,
    Transport,
    HttpStatus,
    ProviderStatus,
    AccountMismatch,
    MalformedResponse,
    MissingField,
    Storage,
    AmbiguousOwnership,
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Withings client error: {self:?}")
    }
}
impl std::error::Error for ClientError {}
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: &'static str,
    pub form: String,
    pub bearer: Option<Secret>,
}
impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("url", &self.url)
            .field("form", &"[REDACTED]")
            .field("bearer", &self.bearer.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}
impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body", &"[REDACTED]")
            .finish()
    }
}
pub trait HttpTransport {
    type Error;
    fn post(&mut self, request: HttpRequest) -> Result<HttpResponse, Self::Error>;
}
/// Must be implemented by a host encrypted durable secret facility; no plain KV fallback.
pub trait SecretStore {
    type Error;
    fn replace_tokens(&mut self, tokens: &TokenSet) -> Result<(), Self::Error>;
}
#[derive(Clone, PartialEq, Eq)]
pub struct TokenSet {
    pub user_id: u64,
    pub access_token: Secret,
    pub refresh_token: Secret,
    pub expires_in: u64,
}
impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet")
            .field("user_id", &"[REDACTED]")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct PendingAuth {
    state: Secret,
    expires_at: i64,
    consumed: bool,
}
impl fmt::Debug for PendingAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingAuth")
            .field("state", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("consumed", &self.consumed)
            .finish()
    }
}
impl PendingAuth {
    /// Host must supply unpredictable CSPRNG-generated URL-safe state (32..=256 bytes).
    pub fn new(state: Secret, expires_at: i64) -> Result<Self, ClientError> {
        if expires_at <= 0
            || !(32..=256).contains(&state.expose_secret().len())
            || !valid_state(state.expose_secret())
        {
            return Err(ClientError::InvalidInput);
        }
        Ok(Self {
            state,
            expires_at,
            consumed: false,
        })
    }
    pub fn consume_callback(&mut self, returned: &str, now: i64) -> Result<(), ClientError> {
        if self.consumed {
            return Err(ClientError::StateReplayed);
        }
        if now < 0 || !valid_state(returned) {
            return Err(ClientError::InvalidInput);
        }
        if now >= self.expires_at {
            self.consumed = true;
            return Err(ClientError::StateExpired);
        }
        if !constant_time_eq(self.state.expose_secret().as_bytes(), returned.as_bytes()) {
            return Err(ClientError::StateMismatch);
        }
        self.consumed = true;
        Ok(())
    }
}
fn valid_state(s: &str) -> bool {
    (32..=256).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~')
}
pub fn authorization_url(
    client_id: &str,
    callback: &str,
    state: &Secret,
) -> Result<String, ClientError> {
    if client_id.is_empty()
        || client_id.len() > 256
        || valid_callback(callback).is_err()
        || !valid_state(state.expose_secret())
    {
        return Err(ClientError::InvalidInput);
    }
    Ok(format!(
        "{AUTH_URL}?response_type=code&client_id={}&scope={}&redirect_uri={}&state={}",
        query_encode(client_id),
        query_encode(BED_SCOPE),
        query_encode(callback),
        query_encode(state.expose_secret())
    ))
}
pub fn exchange_code<T: HttpTransport, S: SecretStore>(
    t: &mut T,
    s: &mut S,
    client_id: &str,
    client_secret: &Secret,
    code: &Secret,
    callback: &str,
) -> Result<TokenSet, ClientError> {
    if client_id.is_empty() || client_id.len() > 256 || valid_callback(callback).is_err() {
        return Err(ClientError::InvalidInput);
    }
    let f = form(&[
        ("action", "requesttoken"),
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("client_secret", client_secret.expose_secret()),
        ("code", code.expose_secret()),
        ("redirect_uri", callback),
    ]);
    let tokens = token_response(
        t.post(HttpRequest {
            url: TOKEN_URL,
            form: f,
            bearer: None,
        })
        .map_err(|_| ClientError::Transport)?,
        client_id,
    )?;
    s.replace_tokens(&tokens)
        .map_err(|_| ClientError::Storage)?;
    Ok(tokens)
}
/// Exchange an authorization code without persistence for an atomic lifecycle writer.
pub fn exchange_code_unstored<T: HttpTransport>(
    t: &mut T,
    client_id: &str,
    client_secret: &Secret,
    code: &Secret,
    callback: &str,
) -> Result<TokenSet, ClientError> {
    if client_id.is_empty() || client_id.len() > 256 || valid_callback(callback).is_err() {
        return Err(ClientError::InvalidInput);
    }
    let f = form(&[
        ("action", "requesttoken"),
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("client_secret", client_secret.expose_secret()),
        ("code", code.expose_secret()),
        ("redirect_uri", callback),
    ]);
    token_response(
        t.post(HttpRequest {
            url: TOKEN_URL,
            form: f,
            bearer: None,
        })
        .map_err(|_| ClientError::Transport)?,
        client_id,
    )
}
pub fn refresh<T: HttpTransport, S: SecretStore>(
    t: &mut T,
    s: &mut S,
    client_id: &str,
    client_secret: &Secret,
    refresh_token: &Secret,
) -> Result<TokenSet, ClientError> {
    if client_id.is_empty() || client_id.len() > 256 {
        return Err(ClientError::InvalidInput);
    }
    let f = form(&[
        ("action", "requesttoken"),
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("client_secret", client_secret.expose_secret()),
        ("refresh_token", refresh_token.expose_secret()),
    ]);
    let tokens = token_response(
        t.post(HttpRequest {
            url: TOKEN_URL,
            form: f,
            bearer: None,
        })
        .map_err(|_| ClientError::Transport)?,
        client_id,
    )?;
    s.replace_tokens(&tokens)
        .map_err(|_| ClientError::Storage)?;
    Ok(tokens)
}
/// Refresh while enforcing account identity before the durable store is called.
/// A foreign-account response is never persisted.
pub fn refresh_bound<T: HttpTransport, S: SecretStore>(
    t: &mut T,
    s: &mut S,
    client_id: &str,
    client_secret: &Secret,
    refresh_token: &Secret,
    expected_user: u64,
) -> Result<TokenSet, ClientError> {
    if expected_user == 0 {
        return Err(ClientError::InvalidInput);
    }
    struct BoundStore<'a, S> {
        inner: &'a mut S,
        expected: u64,
        mismatch: bool,
    }
    impl<S: SecretStore> SecretStore for BoundStore<'_, S> {
        type Error = ();
        fn replace_tokens(&mut self, tokens: &TokenSet) -> Result<(), ()> {
            if tokens.user_id != self.expected {
                self.mismatch = true;
                return Err(());
            }
            self.inner.replace_tokens(tokens).map_err(|_| ())
        }
    }
    let mut bound = BoundStore {
        inner: s,
        expected: expected_user,
        mismatch: false,
    };
    let result = refresh(t, &mut bound, client_id, client_secret, refresh_token);
    if bound.mismatch {
        Err(ClientError::AccountMismatch)
    } else {
        result
    }
}

/// A linked provider device. The opaque ID is redacted from diagnostics.
#[derive(Clone, PartialEq, Eq)]
pub struct Device {
    pub device_id: String,
    pub device_type: String,
    pub model: String,
    pub model_id: u64,
}
impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("device_id", &"[REDACTED]")
            .field("device_type", &self.device_type)
            .field("model", &self.model)
            .field("model_id", &self.model_id)
            .finish()
    }
}

#[derive(Deserialize)]
struct DevicesBody {
    devices: Vec<DeviceWire>,
}
#[derive(Deserialize)]
struct DeviceWire {
    deviceid: String,
    #[serde(rename = "type")]
    device_type: String,
    model: String,
    model_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceSelection {
    Unavailable,
    Candidate(Device),
    Choose(Vec<Device>),
    Selected(Device),
    StaleSelection,
    AccountMismatch,
}

/// Uses the documented `user.getdevice` action and bearer authorization.
pub fn list_devices<T: HttpTransport>(
    t: &mut T,
    token: &Secret,
) -> Result<Vec<Device>, ClientError> {
    let body: DevicesBody = checked_json(
        t.post(HttpRequest {
            url: USER_URL,
            form: form(&[("action", "getdevice")]),
            bearer: Some(token.clone()),
        })
        .map_err(|_| ClientError::Transport)?,
    )?;
    if body.devices.len() > 256 {
        return Err(ClientError::MalformedResponse);
    }
    body.devices
        .into_iter()
        .map(|row| {
            if row.deviceid.is_empty()
                || row.deviceid.len() > 128
                || row.deviceid.bytes().any(|b| b.is_ascii_control())
                || row.device_type.is_empty()
                || row.device_type.len() > 128
                || row.model.is_empty()
                || row.model.len() > 128
                || row.model_id == 0
            {
                return Err(ClientError::MalformedResponse);
            }
            Ok(Device {
                device_id: row.deviceid,
                device_type: row.device_type,
                model: row.model,
                model_id: row.model_id,
            })
        })
        .collect()
}

/// Pure resolution preserves explicit selections and never auto-switches.
pub fn select_sleep_device(
    expected_user: u64,
    authenticated_user: u64,
    discovered: &[Device],
    selected_id: Option<&str>,
) -> DeviceSelection {
    if expected_user == 0 || authenticated_user != expected_user {
        return DeviceSelection::AccountMismatch;
    }
    let mut by_id = std::collections::BTreeMap::<&str, &Device>::new();
    let mut conflicting = std::collections::BTreeSet::new();
    for device in discovered {
        if let Some(prior) = by_id.insert(&device.device_id, device)
            && prior != device
        {
            conflicting.insert(device.device_id.as_str());
        }
    }
    let eligible: Vec<Device> = by_id
        .into_values()
        .filter(|d| d.device_type == "Sleep Monitor" && !conflicting.contains(d.device_id.as_str()))
        .cloned()
        .collect();
    if let Some(id) = selected_id {
        return eligible
            .into_iter()
            .find(|d| d.device_id == id)
            .map_or(DeviceSelection::StaleSelection, DeviceSelection::Selected);
    }
    match eligible.len() {
        0 => DeviceSelection::Unavailable,
        1 => DeviceSelection::Candidate(eligible.into_iter().next().expect("one device")),
        _ => DeviceSelection::Choose(eligible),
    }
}

fn valid_callback(raw: &str) -> Result<(), ClientError> {
    if raw.len() > CALLBACK_MAX_BYTES
        || raw
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        return Err(ClientError::InvalidCallback);
    }
    let u = Url::parse(raw).map_err(|_| ClientError::InvalidCallback)?;
    if u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || u.port().is_some_and(|p| p != 443)
    {
        return Err(ClientError::InvalidCallback);
    }
    let host = u.host().ok_or(ClientError::InvalidCallback)?;
    match host {
        url::Host::Domain(d)
            if d.contains('.') && d != "localhost" && !d.ends_with(".localhost") =>
        {
            Ok(())
        }
        _ => Err(ClientError::InvalidCallback),
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct CallbackUrl(String);
impl CallbackUrl {
    /// Syntax policy only; public DNS, TLS and reachability remain unproven host prerequisites.
    pub fn parse(url: &str) -> Result<Self, ClientError> {
        valid_callback(url)?;
        Ok(Self(url.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for CallbackUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackUrl([REDACTED])")
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct Subscription {
    pub appli: u8,
    pub callback_url: String,
    pub comment: String,
}
impl fmt::Debug for Subscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("appli", &self.appli)
            .field("callback_url", &"[REDACTED]")
            .field("comment", &"[REDACTED]")
            .finish()
    }
}
#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(skip)]
    status: i64,
    body: Option<T>,
}
#[derive(Deserialize)]
struct TokenBody {
    #[serde(deserialize_with = "deserialize_user_id")]
    userid: u64,
    client_id: Option<String>,
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}
fn deserialize_user_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let parsed = match value {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s)
            if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) =>
        {
            s.parse::<u64>().ok()
        }
        _ => None,
    };
    parsed
        .filter(|id| *id > 0)
        .ok_or_else(|| serde::de::Error::custom("invalid account identifier"))
}

#[cfg(test)]
#[test]
fn token_response_accepts_numeric_or_decimal_string_identity() {
    for id in [serde_json::json!(123), serde_json::json!("123")] {
        let body = serde_json::json!({"status":0,"body":{"userid":id,"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600}});
        let tokens = token_response(
            HttpResponse {
                status: 200,
                body: serde_json::to_vec(&body).unwrap(),
            },
            "fixture-client",
        )
        .unwrap();
        assert_eq!(tokens.user_id, 123);
    }
    for id in [
        serde_json::json!(0),
        serde_json::json!("0"),
        serde_json::json!("-1"),
        serde_json::json!("+123"),
        serde_json::json!(" 123"),
        serde_json::json!("1.5"),
        serde_json::json!("18446744073709551616"),
        serde_json::json!(true),
        serde_json::json!(null),
    ] {
        let body = serde_json::json!({"status":0,"body":{"userid":id,"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600}});
        assert!(
            token_response(
                HttpResponse {
                    status: 200,
                    body: serde_json::to_vec(&body).unwrap()
                },
                "fixture-client"
            )
            .is_err()
        );
    }
}

#[derive(Deserialize)]
struct ListBody {
    profiles: Vec<Profile>,
}
#[derive(Deserialize)]
struct Profile {
    appli: u8,
    callbackurl: String,
    #[serde(default)]
    comment: String,
}
fn checked_json<T: for<'de> Deserialize<'de>>(r: HttpResponse) -> Result<T, ClientError> {
    if !(200..300).contains(&r.status) {
        return Err(ClientError::HttpStatus);
    }
    if r.body.len() > MAX_RESPONSE {
        return Err(ClientError::MalformedResponse);
    }
    let v: serde_json::Value =
        serde_json::from_slice(&r.body).map_err(|_| ClientError::MalformedResponse)?;
    if v.get("status").and_then(serde_json::Value::as_i64) != Some(0) {
        return Err(ClientError::ProviderStatus);
    }
    let e: Envelope<T> = serde_json::from_value(v).map_err(|_| ClientError::MalformedResponse)?;
    let _status = e.status;
    e.body.ok_or(ClientError::MissingField)
}
fn token_response(r: HttpResponse, expected_client_id: &str) -> Result<TokenSet, ClientError> {
    let b: TokenBody = checked_json(r)?;
    if b.userid == 0 || b.expires_in == 0 || b.access_token.is_empty() || b.refresh_token.is_empty()
    {
        return Err(ClientError::MissingField);
    }
    if b.client_id
        .as_deref()
        .is_some_and(|c| c != expected_client_id)
    {
        return Err(ClientError::ProviderStatus);
    }
    Ok(TokenSet {
        user_id: b.userid,
        access_token: Secret::new(b.access_token)?,
        refresh_token: Secret::new(b.refresh_token)?,
        expires_in: b.expires_in,
    })
}
fn notify<T: HttpTransport>(
    t: &mut T,
    token: &Secret,
    action: &str,
    fields: &[(&str, &str)],
) -> Result<(), ClientError> {
    let mut v = vec![("action", action)];
    v.extend_from_slice(fields);
    let r = t
        .post(HttpRequest {
            url: NOTIFY_URL,
            form: form(&v),
            bearer: Some(token.clone()),
        })
        .map_err(|_| ClientError::Transport)?;
    if !(200..300).contains(&r.status) {
        return Err(ClientError::HttpStatus);
    }
    if r.body.len() > MAX_RESPONSE {
        return Err(ClientError::MalformedResponse);
    }
    let v: serde_json::Value =
        serde_json::from_slice(&r.body).map_err(|_| ClientError::MalformedResponse)?;
    if v.get("status").and_then(serde_json::Value::as_i64) != Some(0) {
        return Err(ClientError::ProviderStatus);
    }
    let _e: Envelope<serde_json::Value> =
        serde_json::from_value(v).map_err(|_| ClientError::MalformedResponse)?;
    // Successful notify responses include an empty JSON body object in official samples.
    Ok(())
}
pub fn list_subscriptions<T: HttpTransport>(
    t: &mut T,
    token: &Secret,
) -> Result<Vec<Subscription>, ClientError> {
    let mut all = Vec::new();
    for appli in [50, 51] {
        let b: ListBody = checked_json(
            t.post(HttpRequest {
                url: NOTIFY_URL,
                form: form(&[
                    ("action", "list"),
                    ("appli", if appli == 50 { "50" } else { "51" }),
                ]),
                bearer: Some(token.clone()),
            })
            .map_err(|_| ClientError::Transport)?,
        )?;
        all.extend(b.profiles.into_iter().map(|p| Subscription {
            appli: p.appli,
            callback_url: p.callbackurl,
            comment: p.comment,
        }));
    }
    Ok(all)
}
/// Subscribe one supported category. The caller owns reconciliation and readback.
pub fn subscribe_category<T: HttpTransport>(
    t: &mut T,
    token: &Secret,
    callback: &CallbackUrl,
    comment: &str,
    appli: u8,
) -> Result<(), ClientError> {
    if comment.len() > 256 || !(appli == BED_APPLI || appli == BED_APPLI_OUT) {
        return Err(ClientError::InvalidInput);
    }
    notify(
        t,
        token,
        "subscribe",
        &[
            ("callbackurl", callback.as_str()),
            ("appli", if appli == BED_APPLI { "50" } else { "51" }),
            ("comment", comment),
        ],
    )
}
/// Both categories must succeed; errors do not certify a healthy complete subscription.
pub fn subscribe_bed<T: HttpTransport>(
    t: &mut T,
    token: &Secret,
    callback: &CallbackUrl,
    comment: &str,
) -> Result<(), ClientError> {
    for appli in [BED_APPLI, BED_APPLI_OUT] {
        subscribe_category(t, token, callback, comment, appli)?;
    }
    Ok(())
}
/// Withings revoke is keyed by callback URL + appli; comment is not a provider-side scope.
/// Any foreign comment sharing a target tuple makes that tuple ambiguous and blocks all revokes.
pub fn revoke_owned<T: HttpTransport>(
    t: &mut T,
    token: &Secret,
    callback: &CallbackUrl,
    listed: &[Subscription],
    owned_comment: &str,
) -> Result<usize, ClientError> {
    let mut tuples = HashSet::new();
    for r in listed {
        if r.callback_url == callback.as_str() && (r.appli == 50 || r.appli == 51) {
            if r.comment != owned_comment {
                return Err(ClientError::AmbiguousOwnership);
            }
            tuples.insert(r.appli);
        }
    }
    let mut n = 0;
    for appli in [50, 51] {
        if tuples.contains(&appli) {
            notify(
                t,
                token,
                "revoke",
                &[
                    ("callbackurl", callback.as_str()),
                    ("appli", if appli == 50 { "50" } else { "51" }),
                ],
            )?;
            n += 1;
        }
    }
    Ok(n)
}
fn form(v: &[(&str, &str)]) -> String {
    v.iter()
        .map(|(k, x)| format!("{}={}", form_encode(k), form_encode(x)))
        .collect::<Vec<_>>()
        .join("&")
}
fn query_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn form_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"*-._".contains(&b) {
                (b as char).to_string()
            } else if b == b' ' {
                "+".into()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut d = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        d |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    d == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    #[derive(Default)]
    struct Mock {
        req: Vec<HttpRequest>,
        res: VecDeque<HttpResponse>,
    }
    impl HttpTransport for Mock {
        type Error = ();
        fn post(&mut self, r: HttpRequest) -> Result<HttpResponse, ()> {
            self.req.push(r);
            self.res.pop_front().ok_or(())
        }
    }
    #[derive(Default)]
    struct Store {
        saved: Vec<TokenSet>,
        fail: bool,
    }
    impl SecretStore for Store {
        type Error = ();
        fn replace_tokens(&mut self, t: &TokenSet) -> Result<(), ()> {
            if self.fail {
                Err(())
            } else {
                self.saved.push(t.clone());
                Ok(())
            }
        }
    }
    fn r(status: u16, b: &str) -> HttpResponse {
        HttpResponse {
            status,
            body: b.as_bytes().into(),
        }
    }
    const TOK: &str = r#"{"status":0,"body":{"userid":123,"access_token":"synthetic-access","refresh_token":"synthetic-refresh","expires_in":10800}}"#;
    const CB: &str = "https://example.org/cb";
    const STATE: &str = "0123456789abcdef0123456789abcdef";
    fn ok() -> HttpResponse {
        r(200, r#"{"status":0,"body":{}}"#)
    }
    #[test]
    fn auth_scope_encoding_and_callback_validation() {
        let u = authorization_url("client id", CB, &Secret::new(STATE).unwrap()).unwrap();
        assert!(u.contains("scope=user.sleepevents"));
        assert!(u.contains("client_id=client%20id"));
        for bad in [
            "http://example.org",
            "https://localhost",
            "https://127.0.0.1",
            "https://2130706433",
            "https://[::1]",
            "https://example.org:444",
            "https://u:p@example.org",
            "https://example.org/#frag",
            "https://example.org/ bad",
        ] {
            assert!(CallbackUrl::parse(bad).is_err(), "{bad}");
        }
        assert!(CallbackUrl::parse("https://example.org:443/cb").is_ok());
    }
    #[test]
    fn csrf_mismatch_does_not_consume_valid_state_then_single_use() {
        let mut p = PendingAuth::new(Secret::new(STATE).unwrap(), 10).unwrap();
        assert_eq!(
            p.consume_callback("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1),
            Err(ClientError::StateMismatch)
        );
        assert_eq!(p.consume_callback(STATE, 1), Ok(()));
        assert_eq!(
            p.consume_callback(STATE, 1),
            Err(ClientError::StateReplayed)
        );
        assert_eq!(
            PendingAuth::new(Secret::new("state").unwrap(), 10).unwrap_err(),
            ClientError::InvalidInput
        );
        assert_eq!(
            p.consume_callback(STATE, -1),
            Err(ClientError::StateReplayed)
        );
    }
    #[test]
    fn csrf_negative_time_and_expiry_policy() {
        let mut p = PendingAuth::new(Secret::new(STATE).unwrap(), 10).unwrap();
        assert_eq!(
            p.consume_callback(STATE, -1),
            Err(ClientError::InvalidInput)
        );
        assert_eq!(
            p.consume_callback(STATE, 10),
            Err(ClientError::StateExpired)
        );
        assert_eq!(
            p.consume_callback(STATE, 1),
            Err(ClientError::StateReplayed)
        );
    }
    #[test]
    fn redaction_and_secret_access_contract() {
        let secret = Secret::new("synthetic-secret-value").unwrap();
        assert_eq!(secret.expose_secret(), "synthetic-secret-value");
        assert!(!format!("{secret:?}").contains("synthetic-secret-value"));
        let response = r(200, "synthetic-token-in-response");
        assert!(!format!("{response:?}").contains("synthetic-token-in-response"));
        let cb = CallbackUrl::parse("https://example.org/synthetic-route-secret").unwrap();
        assert!(!format!("{cb:?}").contains("synthetic-route-secret"));
        let sub = Subscription {
            appli: 50,
            callback_url: cb.as_str().into(),
            comment: "synthetic-private-comment".into(),
        };
        assert!(!format!("{sub:?}").contains("synthetic-private-comment"));
    }
    #[test]
    fn list_queries_both_categories() {
        let mut t = Mock::default();
        for _ in 0..2 {
            t.res
                .push_back(r(200, r#"{"status":0,"body":{"profiles":[]}}"#));
        }
        assert!(
            list_subscriptions(&mut t, &Secret::new("token").unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(t.req[0].form.contains("appli=50"));
        assert!(t.req[1].form.contains("appli=51"));
    }
    #[test]
    fn documented_getdevice_and_bounded_account_selection() {
        let fixture = |rows: &str| format!(r#"{{"status":0,"body":{{"devices":[{rows}]}}}}"#);
        let mat = r#"{"deviceid":"synthetic-sleep-id","type":"Sleep Monitor","model":"Aura Sensor","model_id":61}"#;
        let scale =
            r#"{"deviceid":"synthetic-scale-id","type":"Scale","model":"Body+","model_id":5}"#;
        let other = r#"{"deviceid":"synthetic-unknown-id","type":"Uncatalogued","model":"Unknown","model_id":999}"#;
        let mut t = Mock::default();
        t.res
            .push_back(r(200, &fixture(&format!("{scale},{mat},{other}"))));
        let devices = list_devices(&mut t, &Secret::new("synthetic-token").unwrap()).unwrap();
        assert_eq!(t.req[0].url, USER_URL);
        assert_eq!(t.req[0].form, "action=getdevice");
        assert_eq!(devices.len(), 3);
        assert_eq!(
            select_sleep_device(7, 7, &[], None),
            DeviceSelection::Unavailable
        );
        assert!(matches!(
            select_sleep_device(7, 7, &devices, None),
            DeviceSelection::Candidate(_)
        ));
        assert_eq!(
            select_sleep_device(7, 8, &devices, None),
            DeviceSelection::AccountMismatch
        );
        assert_eq!(
            select_sleep_device(7, 7, &devices, Some("vanished")),
            DeviceSelection::StaleSelection
        );
        assert!(!format!("{:?}", devices[1]).contains("synthetic-sleep-id"));
        let mut conflicting = devices.clone();
        conflicting.push(Device {
            device_type: "Scale".into(),
            ..devices[1].clone()
        });
        for rows in [conflicting.clone(), conflicting.into_iter().rev().collect()] {
            assert_eq!(
                select_sleep_device(7, 7, &rows, Some("synthetic-sleep-id")),
                DeviceSelection::StaleSelection
            );
        }
        let mut duplicate = devices.clone();
        duplicate.push(devices[1].clone());
        assert!(matches!(
            select_sleep_device(7, 7, &duplicate, Some("synthetic-sleep-id")),
            DeviceSelection::Selected(_)
        ));
        duplicate.push(Device {
            device_id: "synthetic-second".into(),
            device_type: "Sleep Monitor".into(),
            model: "Aura Sensor".into(),
            model_id: 61,
        });
        assert!(
            matches!(select_sleep_device(7, 7, &duplicate, None), DeviceSelection::Choose(v) if v.len() == 2)
        );
    }
    #[test]
    fn getdevice_rejects_bad_provider_replies_and_transport_without_leaking() {
        for (status, body, expected) in [
            (
                503,
                r#"{"status":0,"body":{"devices":[]}}"#,
                ClientError::HttpStatus,
            ),
            (200, "not-json", ClientError::MalformedResponse),
            (
                200,
                r#"{"status":12,"body":{"devices":[]}}"#,
                ClientError::ProviderStatus,
            ),
            (
                200,
                r#"{"status":0,"body":{}}"#,
                ClientError::MalformedResponse,
            ),
            (
                200,
                r#"{"status":0,"body":{"devices":[{"deviceid":"x","type":"Sleep Monitor","model":"Aura","model_id":0}]}}"#,
                ClientError::MalformedResponse,
            ),
        ] {
            let mut t = Mock::default();
            t.res.push_back(r(status, body));
            assert_eq!(
                list_devices(&mut t, &Secret::new("synthetic-token").unwrap()),
                Err(expected)
            );
        }
        let mut t = Mock::default();
        assert_eq!(
            list_devices(&mut t, &Secret::new("synthetic-token").unwrap()),
            Err(ClientError::Transport)
        );
        let mut t = Mock::default();
        t.res.push_back(HttpResponse {
            status: 200,
            body: vec![b' '; MAX_RESPONSE + 1],
        });
        assert_eq!(
            list_devices(&mut t, &Secret::new("synthetic-token").unwrap()),
            Err(ClientError::MalformedResponse)
        );
    }
    #[test]
    fn subscribe_both_and_partial_failure_is_error() {
        let mut t = Mock::default();
        t.res.extend([ok(), ok()]);
        subscribe_bed(
            &mut t,
            &Secret::new("token").unwrap(),
            &CallbackUrl::parse(CB).unwrap(),
            "own",
        )
        .unwrap();
        assert_eq!(t.req.len(), 2);
        assert!(t.req[0].form.contains("appli=50"));
        assert!(t.req[1].form.contains("appli=51"));
        let mut partial = Mock::default();
        partial.res.push_back(ok());
        partial.res.push_back(r(200, r#"{"status":5,"body":{}}"#));
        assert_eq!(
            subscribe_bed(
                &mut partial,
                &Secret::new("token").unwrap(),
                &CallbackUrl::parse(CB).unwrap(),
                "own"
            ),
            Err(ClientError::ProviderStatus)
        );
        assert_eq!(partial.req.len(), 2);
    }
    #[test]
    fn revoke_dedupes_owned_50_51_and_refuses_shared_or_foreign_tuple() {
        let cb = CallbackUrl::parse(CB).unwrap();
        let tok = Secret::new("token").unwrap();
        let rows = [
            Subscription {
                appli: 50,
                callback_url: CB.into(),
                comment: "own".into(),
            },
            Subscription {
                appli: 50,
                callback_url: CB.into(),
                comment: "own".into(),
            },
            Subscription {
                appli: 51,
                callback_url: CB.into(),
                comment: "own".into(),
            },
            Subscription {
                appli: 50,
                callback_url: "https://foreign.org".into(),
                comment: "foreign".into(),
            },
        ];
        let mut t = Mock::default();
        t.res.extend([ok(), ok()]);
        assert_eq!(revoke_owned(&mut t, &tok, &cb, &rows, "own"), Ok(2));
        assert_eq!(t.req.len(), 2);
        assert!(t.req[0].form.contains("appli=50"));
        assert!(t.req[1].form.contains("appli=51"));
        let mut conflict = rows.to_vec();
        conflict.push(Subscription {
            appli: 51,
            callback_url: CB.into(),
            comment: "foreign".into(),
        });
        let mut t = Mock::default();
        assert_eq!(
            revoke_owned(&mut t, &tok, &cb, &conflict, "own"),
            Err(ClientError::AmbiguousOwnership)
        );
        assert!(t.req.is_empty());
    }
    #[test]
    fn revoke_partial_failure_is_not_reported_as_complete() {
        let cb = CallbackUrl::parse(CB).unwrap();
        let rows = [50, 51].map(|appli| Subscription {
            appli,
            callback_url: CB.into(),
            comment: "own".into(),
        });
        let mut t = Mock::default();
        t.res.push_back(ok());
        t.res.push_back(r(200, r#"{"status":4,"body":{}}"#));
        assert_eq!(
            revoke_owned(&mut t, &Secret::new("token").unwrap(), &cb, &rows, "own"),
            Err(ClientError::ProviderStatus)
        );
        assert_eq!(t.req.len(), 2);
    }
    #[test]
    fn token_status_malformed_store_rotation_and_client_binding_fail_closed() {
        for (status, body, error) in [
            (503, TOK, ClientError::HttpStatus),
            (
                200,
                r#"{"status":9,"body":{}}"#,
                ClientError::ProviderStatus,
            ),
            (200, "bad", ClientError::MalformedResponse),
        ] {
            let mut t = Mock::default();
            t.res.push_back(r(status, body));
            assert_eq!(
                exchange_code(
                    &mut t,
                    &mut Store::default(),
                    "id",
                    &Secret::new("sec").unwrap(),
                    &Secret::new("code").unwrap(),
                    CB
                )
                .unwrap_err(),
                error
            );
        }
        let mut t = Mock::default();
        t.res.push_back(r(200, r#"{"status":0,"body":{"userid":1,"client_id":"wrong","access_token":"x","refresh_token":"y","expires_in":1}}"#));
        assert_eq!(
            refresh(
                &mut t,
                &mut Store::default(),
                "id",
                &Secret::new("sec").unwrap(),
                &Secret::new("old").unwrap()
            )
            .unwrap_err(),
            ClientError::ProviderStatus
        );
    }
    #[test]
    fn exchange_and_refresh_persist_before_success() {
        let mut t = Mock::default();
        t.res.push_back(r(200, TOK));
        let mut s = Store::default();
        assert_eq!(
            exchange_code(
                &mut t,
                &mut s,
                "id",
                &Secret::new("sec").unwrap(),
                &Secret::new("code").unwrap(),
                CB
            )
            .unwrap()
            .user_id,
            123
        );
        assert_eq!(s.saved.len(), 1);
        let mut t = Mock::default();
        t.res.push_back(r(200, TOK));
        let mut s = Store {
            fail: true,
            ..Store::default()
        };
        assert_eq!(
            refresh(
                &mut t,
                &mut s,
                "id",
                &Secret::new("sec").unwrap(),
                &Secret::new("old").unwrap()
            )
            .unwrap_err(),
            ClientError::Storage
        );
    }
    #[test]
    fn fixture_is_synthetic() {
        assert!(TOK.contains("synthetic"));
    }
}
