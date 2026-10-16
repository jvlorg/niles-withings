//! Stateful owner for account-bound setup and webhook adapter activation.
use crate::{
    adapter::{Adapter, Sink, Store},
    application::{
        self, Binding, SecureAccountRecord, SelectionStore, SetupResolution, WebhookRequest,
        WebhookResponse,
    },
    lifecycle::Status as LifecycleStatus,
    withings_client::HttpTransport,
};
use std::{fmt, marker::PhantomData};

/// A host-provided key/value backend. The coordinator supplies the complete,
/// versioned account/device key; implementations must not reinterpret it.
pub trait ScopedBackend {
    type Error: std::fmt::Display;
    fn load_value(&mut self, key: &str) -> Result<Option<String>, Self::Error>;
    fn save_value(&mut self, key: &str, value: &str) -> Result<(), Self::Error>;
}

/// Versioned, collision-free storage identity. Byte-length prefixes preserve
/// exact UTF-8 identity boundaries without embedding credentials or tokens.
pub fn presence_namespace(account: &str, device: &str) -> Result<String, &'static str> {
    if account.is_empty()
        || device.is_empty()
        || account.len() > 128
        || device.len() > 128
        || account.bytes().any(|b| b.is_ascii_control())
        || device.bytes().any(|b| b.is_ascii_control())
    {
        return Err("invalid presence identity");
    }
    Ok(format!(
        "niles-withings:presence:v1:{}:{account}:{}:{device}",
        account.len(),
        device.len()
    ))
}

pub struct ScopedStore<B> {
    backend: B,
    key: String,
}
impl<B> ScopedStore<B> {
    pub fn new(backend: B, account: &str, device: &str) -> Result<Self, &'static str> {
        Ok(Self {
            backend,
            key: presence_namespace(account, device)?,
        })
    }
    pub fn into_backend(self) -> B {
        self.backend
    }
    pub fn key(&self) -> &str {
        &self.key
    }
}
impl<B: ScopedBackend> Store for ScopedStore<B> {
    type Error = B::Error;
    fn load(&mut self) -> Result<Option<String>, Self::Error> {
        self.backend.load_value(&self.key)
    }
    fn save(&mut self, state: &str) -> Result<(), Self::Error> {
        self.backend.save_value(&self.key, state)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupStatus {
    AccountRequired,
    SelectionNeeded,
    SelectionStale,
    SubscriptionPending,
    Configured,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountRecoveryStatus {
    NotChecked,
    Validated,
    ReconnectRequired,
    InterruptedRefresh,
}

/// Host-neutral setup presentation derived from coordinator ownership.
#[derive(Clone, PartialEq, Eq)]
pub struct SetupView {
    pub connection: ConnectionState,
    pub discovery: DiscoveryState,
    pub candidates: Vec<SetupCandidate>,
    pub selected: Option<SetupCandidate>,
    pub subscription: SubscriptionState,
    pub occupancy: OccupancyState,
    pub actions: Vec<SetupAction>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryState {
    NotStarted,
    InterruptedRefresh,
    AuthorizationRequired,
    NoEligibleMats,
    Unavailable,
    Candidate,
    MultipleCandidates,
    StaleSelection,
    Selected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionState {
    NotConfigured,
    Incomplete,
    Ready,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccupancyState {
    Unknown,
}
#[derive(Clone, PartialEq, Eq)]
pub struct SetupCandidate {
    pub label: String,
    action_id: String,
}
impl SetupCandidate {
    pub fn action_id(&self) -> &str {
        &self.action_id
    }
}
#[derive(Clone, PartialEq, Eq)]
pub enum SetupAction {
    Connect,
    Reconnect,
    RetryDiscovery,
    AcceptCandidate { action_id: String, generation: u64 },
    ChooseCandidate { action_id: String, generation: u64 },
    SwitchMat,
    RefreshDiscovery,
}
impl fmt::Debug for SetupCandidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetupCandidate")
            .field("label", &self.label)
            .field("action_id", &"[REDACTED]")
            .finish()
    }
}
impl fmt::Debug for SetupAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AcceptCandidate { generation, .. } => f
                .debug_struct("AcceptCandidate")
                .field("action_id", &"[REDACTED]")
                .field("generation", generation)
                .finish(),
            Self::ChooseCandidate { generation, .. } => f
                .debug_struct("ChooseCandidate")
                .field("action_id", &"[REDACTED]")
                .field("generation", generation)
                .finish(),
            Self::Connect => f.write_str("Connect"),
            Self::Reconnect => f.write_str("Reconnect"),
            Self::RetryDiscovery => f.write_str("RetryDiscovery"),
            Self::SwitchMat => f.write_str("SwitchMat"),
            Self::RefreshDiscovery => f.write_str("RefreshDiscovery"),
        }
    }
}
impl fmt::Debug for SetupView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetupView")
            .field("connection", &self.connection)
            .field("discovery", &self.discovery)
            .field("candidates", &self.candidates)
            .field("selected", &self.selected)
            .field("subscription", &self.subscription)
            .field("occupancy", &self.occupancy)
            .field("actions", &self.actions)
            .finish()
    }
}

/// The current authenticated account, discovery generation, durable selection, and adapter form one ownership boundary.
pub struct Coordinator<S: Store, H: Sink> {
    adapter: Option<Adapter<S, H>>,
    account: Option<u64>,
    generation: u64,
    discovered: Option<(u64, SetupResolution)>,
    binding: Option<Binding>,
    setup: SetupStatus,
    lifecycle_ready: bool,
    choosing: bool,
    discovery_failed: bool,
    recovery: AccountRecoveryStatus,
    marker: PhantomData<(S, H)>,
}
impl<S: Store, H: Sink> Default for Coordinator<S, H> {
    fn default() -> Self {
        Self {
            adapter: None,
            account: None,
            generation: 0,
            discovered: None,
            binding: None,
            setup: SetupStatus::AccountRequired,
            lifecycle_ready: false,
            choosing: false,
            discovery_failed: false,
            recovery: AccountRecoveryStatus::NotChecked,
            marker: PhantomData,
        }
    }
}
impl<B: ScopedBackend, H: Sink> Coordinator<ScopedStore<B>, H> {
    pub fn accept_scoped<P: SelectionStore>(
        &mut self,
        store: &mut P,
        device_id: &str,
        expected_generation: u64,
        backend: B,
        sink: H,
    ) -> Result<(), application::ApplicationError> {
        let account = self
            .account
            .ok_or(application::ApplicationError::InvalidSelection)?;
        if expected_generation != self.generation {
            return Err(application::ApplicationError::InvalidSelection);
        }
        let key = presence_namespace(&account.to_string(), device_id)
            .map_err(|_| application::ApplicationError::InvalidSelection)?;
        self.accept(
            store,
            device_id,
            expected_generation,
            ScopedStore { backend, key },
            sink,
        )
    }
    pub fn activate_scoped<P: SelectionStore>(
        &mut self,
        store: &mut P,
        account: u64,
        backend: B,
        sink: H,
    ) -> Result<(), application::ApplicationError> {
        let device = match self.discovered.as_ref() {
            Some((generation, SetupResolution::Selected(binding)))
                if *generation == self.generation
                    && self.account == Some(account)
                    && binding.identity().user_id == account.to_string() =>
            {
                binding.identity().device_id
            }
            _ => return Err(application::ApplicationError::InvalidSelection),
        };
        let scoped = ScopedStore::new(backend, &account.to_string(), &device)
            .map_err(|_| application::ApplicationError::InvalidSelection)?;
        self.activate(store, account, scoped, sink)
    }
}

impl<S: Store, H: Sink> Coordinator<S, H> {
    pub fn status(&self) -> SetupStatus {
        self.setup
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn restore_account(
        &mut self,
        serialized: Option<&str>,
        account: u64,
        now: i64,
    ) -> Result<
        (
            crate::withings_client::TokenSet,
            crate::lifecycle::TokenMetadata,
        ),
        application::ApplicationError,
    > {
        // Explicit restoration always revokes prior runtime ownership, even for the same account.
        self.begin_account(Some(account));
        self.adapter = None;
        self.binding = None;
        self.discovered = None;
        self.lifecycle_ready = false;
        self.generation = self.generation.wrapping_add(1);
        self.recovery = AccountRecoveryStatus::ReconnectRequired;
        self.setup = SetupStatus::AccountRequired;
        let raw = serialized.ok_or(application::ApplicationError::InvalidRecord)?;
        let restored = SecureAccountRecord::restore(raw, account, now).inspect_err(|error| {
            if *error == application::ApplicationError::RecoveryRequired {
                self.recovery = AccountRecoveryStatus::InterruptedRefresh;
            }
        })?;
        self.recovery = AccountRecoveryStatus::Validated;
        self.setup = SetupStatus::SelectionNeeded;
        Ok(restored)
    }
    pub fn presentation(&self) -> SetupView {
        use crate::application::SetupResolution;
        let pending_recovery = matches!(
            self.recovery,
            AccountRecoveryStatus::ReconnectRequired | AccountRecoveryStatus::InterruptedRefresh
        );
        let resolution = self
            .discovered
            .as_ref()
            .filter(|(g, _)| *g == self.generation)
            .map(|(_, r)| r);
        let connection = if self.account.is_some() {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        };
        let mut candidates = Vec::new();
        let mut selected = self
            .binding
            .as_ref()
            .filter(|_| !pending_recovery)
            .map(|binding| SetupCandidate {
                label: "Selected mat".into(),
                action_id: binding.identity().device_id,
            });
        let discovery = if pending_recovery {
            if self.recovery == AccountRecoveryStatus::InterruptedRefresh {
                DiscoveryState::InterruptedRefresh
            } else {
                DiscoveryState::AuthorizationRequired
            }
        } else {
            match resolution {
                None if selected.is_some() => DiscoveryState::Selected,
                None => DiscoveryState::NotStarted,
                Some(SetupResolution::Unavailable) => {
                    if self.binding.is_some() {
                        DiscoveryState::Selected
                    } else {
                        if self.discovery_failed {
                            DiscoveryState::Unavailable
                        } else {
                            DiscoveryState::NoEligibleMats
                        }
                    }
                }
                Some(SetupResolution::Candidate { device, .. }) => {
                    candidates.push(SetupCandidate {
                        label: format!("{} {}", device.model, device.device_type),
                        action_id: device.device_id.clone(),
                    });
                    DiscoveryState::Candidate
                }
                Some(SetupResolution::Choose { devices, .. }) => {
                    candidates.extend(devices.iter().map(|device| SetupCandidate {
                        label: format!("{} {}", device.model, device.device_type),
                        action_id: device.device_id.clone(),
                    }));
                    DiscoveryState::MultipleCandidates
                }
                Some(SetupResolution::StaleSelection) => DiscoveryState::StaleSelection,
                Some(SetupResolution::Selected(binding)) => {
                    selected = Some(SetupCandidate {
                        label: "Selected mat".into(),
                        action_id: binding.identity().device_id,
                    });
                    DiscoveryState::Selected
                }
            }
        };
        let subscription = if pending_recovery || self.binding.is_none() {
            SubscriptionState::NotConfigured
        } else if self.lifecycle_ready {
            SubscriptionState::Ready
        } else {
            SubscriptionState::Incomplete
        };
        let mut actions = Vec::new();
        if pending_recovery {
            actions.push(SetupAction::Reconnect);
        } else if self.account.is_none() {
            actions.push(SetupAction::Connect);
        } else {
            match discovery {
                DiscoveryState::Unavailable | DiscoveryState::NoEligibleMats => {
                    actions.push(SetupAction::RetryDiscovery)
                }
                DiscoveryState::Candidate => {
                    if let Some(c) = candidates.first() {
                        actions.push(SetupAction::AcceptCandidate {
                            action_id: c.action_id.clone(),
                            generation: self.generation,
                        });
                    }
                }
                DiscoveryState::MultipleCandidates => {
                    actions.extend(candidates.iter().map(|c| SetupAction::ChooseCandidate {
                        action_id: c.action_id.clone(),
                        generation: self.generation,
                    }))
                }
                DiscoveryState::StaleSelection => actions.push(SetupAction::SwitchMat),
                DiscoveryState::Selected => {
                    actions.extend([SetupAction::RefreshDiscovery, SetupAction::SwitchMat])
                }
                _ => actions.push(SetupAction::RetryDiscovery),
            }
        }
        SetupView {
            connection,
            discovery,
            candidates,
            selected,
            subscription,
            occupancy: OccupancyState::Unknown,
            actions,
        }
    }
    pub fn begin_account(&mut self, account: Option<u64>) {
        let account = account.filter(|id| *id != 0);
        if self.account == account {
            return;
        }
        if self.account != account {
            self.adapter = None;
            self.binding = None;
            self.discovered = None;
            self.lifecycle_ready = false;
            self.generation = self.generation.wrapping_add(1);
            self.recovery = if account.is_some() {
                AccountRecoveryStatus::ReconnectRequired
            } else {
                AccountRecoveryStatus::NotChecked
            };
        }
        self.choosing = false;
        self.account = account;
        self.setup = if account.is_some() {
            SetupStatus::SelectionNeeded
        } else {
            SetupStatus::AccountRequired
        };
    }
    pub fn switch_device(&mut self) {
        self.choosing = true;
        self.adapter = None;
        self.binding = None;
        self.discovered = None;
        self.lifecycle_ready = false;
        self.generation = self.generation.wrapping_add(1);
        self.setup = if self.account.is_some() {
            SetupStatus::SelectionNeeded
        } else {
            SetupStatus::AccountRequired
        };
    }
    pub fn discovery_failed(&mut self) {
        self.adapter = None;
        self.binding = None;
        self.discovered = None;
        self.lifecycle_ready = false;
        self.generation = self.generation.wrapping_add(1);
        self.discovery_failed = true;
        self.discovered = Some((self.generation, application::SetupResolution::Unavailable));
        self.setup = if self.account.is_some() {
            SetupStatus::SelectionNeeded
        } else {
            SetupStatus::AccountRequired
        };
    }
    pub fn discover<T: HttpTransport, P: SelectionStore>(
        &mut self,
        transport: &mut T,
        store: &mut P,
        record: &SecureAccountRecord,
        now: i64,
    ) -> Result<SetupStatus, application::ApplicationError> {
        if self.recovery != AccountRecoveryStatus::Validated {
            return Err(application::ApplicationError::RecoveryRequired);
        }
        let Some(account) = self.account else {
            self.discovery_failed();
            return Ok(self.setup);
        };
        self.adapter = None;
        self.binding = None;
        self.lifecycle_ready = false;
        self.generation = self.generation.wrapping_add(1);
        struct View<'a, P>(&'a mut P, bool);
        impl<P: SelectionStore> SelectionStore for View<'_, P> {
            type Error = P::Error;
            fn load_selection(&mut self) -> Result<Option<String>, Self::Error> {
                if self.1 {
                    Ok(None)
                } else {
                    self.0.load_selection()
                }
            }
            fn save_selection(&mut self, s: &str) -> Result<(), Self::Error> {
                self.0.save_selection(s)
            }
        }
        let result = match application::setup_discover(
            transport,
            &mut View(store, self.choosing),
            record,
            account,
            now,
        ) {
            Ok(result) => result,
            Err(error) => {
                self.discovery_failed = true;
                self.discovered = Some((self.generation, SetupResolution::Unavailable));
                self.setup = SetupStatus::SelectionNeeded;
                return Err(error);
            }
        };
        self.discovery_failed = false;
        self.setup = match &result {
            SetupResolution::Unavailable
            | SetupResolution::Candidate { .. }
            | SetupResolution::Choose { .. } => SetupStatus::SelectionNeeded,
            SetupResolution::StaleSelection => SetupStatus::SelectionStale,
            SetupResolution::Selected(_) => SetupStatus::SubscriptionPending,
        };
        if let SetupResolution::Selected(binding) = &result {
            self.binding = Some(binding.clone());
        }
        self.discovered = Some((self.generation, result));
        Ok(self.setup)
    }
    fn accept<P: SelectionStore>(
        &mut self,
        store: &mut P,
        device_id: &str,
        expected_generation: u64,
        sink_store: S,
        sink: H,
    ) -> Result<(), application::ApplicationError> {
        if self.recovery != AccountRecoveryStatus::Validated {
            return Err(application::ApplicationError::RecoveryRequired);
        }
        let Some(account) = self.account else {
            return Err(application::ApplicationError::InvalidSelection);
        };
        let Some((generation, discovery)) = self.discovered.as_ref() else {
            return Err(application::ApplicationError::InvalidSelection);
        };
        if *generation != self.generation || expected_generation != self.generation {
            return Err(application::ApplicationError::InvalidSelection);
        }
        self.adapter = None;
        self.lifecycle_ready = false;
        self.binding = None;
        self.setup = SetupStatus::SelectionNeeded;
        let binding = application::accept_selection(store, account, discovery, device_id)?;
        let adapter = application::adapter_from_binding(&binding, sink_store, sink)
            .map_err(|_| application::ApplicationError::Delivery)?;
        self.choosing = false;
        self.binding = Some(binding);
        self.adapter = Some(adapter);
        self.setup = SetupStatus::SubscriptionPending;
        self.discovered = None;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    /// Attach the adapter only after durable selection has been read back and account-bound.
    fn activate<P: SelectionStore>(
        &mut self,
        store: &mut P,
        account: u64,
        sink_store: S,
        sink: H,
    ) -> Result<(), application::ApplicationError> {
        self.adapter = None;
        self.binding = None;
        self.lifecycle_ready = false;
        if self.recovery != AccountRecoveryStatus::Validated {
            return Err(application::ApplicationError::RecoveryRequired);
        }
        if self.account != Some(account) {
            return Err(application::ApplicationError::InvalidSelection);
        }
        self.setup = SetupStatus::SelectionNeeded;
        let Some((generation, SetupResolution::Selected(verified))) = self.discovered.as_ref()
        else {
            return Err(application::ApplicationError::InvalidSelection);
        };
        if *generation != self.generation {
            return Err(application::ApplicationError::InvalidSelection);
        }
        let raw = store
            .load_selection()
            .map_err(|_| application::ApplicationError::Storage)?
            .ok_or(application::ApplicationError::InvalidSelection)?;
        let selection = application::Selection::restore(&raw)?;
        let binding = selection.binding_for(account)?;
        if &binding != verified {
            return Err(application::ApplicationError::InvalidSelection);
        }
        let adapter = application::adapter_from_binding(&binding, sink_store, sink)
            .map_err(|_| application::ApplicationError::Delivery)?;
        self.choosing = false;
        self.binding = Some(binding);
        self.adapter = Some(adapter);
        self.setup = SetupStatus::SubscriptionPending;
        Ok(())
    }
    pub fn set_lifecycle(&mut self, status: LifecycleStatus) {
        self.lifecycle_ready = status == LifecycleStatus::Ready;
        self.setup = if self.adapter.is_some() && self.lifecycle_ready {
            SetupStatus::Configured
        } else if self.adapter.is_some() {
            SetupStatus::SubscriptionPending
        } else {
            self.setup
        };
    }
    pub fn webhook(
        &mut self,
        request: &WebhookRequest,
        now: i64,
        max_age: i64,
        future_skew: i64,
    ) -> WebhookResponse {
        match self.adapter.as_mut() {
            Some(adapter) => application::on_webhook(adapter, request, now, max_age, future_skew),
            None => WebhookResponse {
                status: 503,
                body: Vec::new(),
            },
        }
    }
}
/// Convenience helper for creating a restored account record from an encrypted host value.
pub fn restore_record(
    serialized: &str,
    account: u64,
    now: i64,
) -> Result<
    (
        crate::withings_client::TokenSet,
        crate::lifecycle::TokenMetadata,
    ),
    application::ApplicationError,
> {
    SecureAccountRecord::restore(serialized, account, now)
}
