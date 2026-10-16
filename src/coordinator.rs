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
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupStatus {
    AccountRequired,
    SelectionNeeded,
    SelectionStale,
    SubscriptionPending,
    Configured,
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
            marker: PhantomData,
        }
    }
}
impl<S: Store, H: Sink> Coordinator<S, H> {
    pub fn status(&self) -> SetupStatus {
        self.setup
    }
    pub fn generation(&self) -> u64 {
        self.generation
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
        }
        self.account = account;
        self.setup = if account.is_some() {
            SetupStatus::SelectionNeeded
        } else {
            SetupStatus::AccountRequired
        };
    }
    pub fn switch_device(&mut self) {
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
        let Some(account) = self.account else {
            self.discovery_failed();
            return Ok(self.setup);
        };
        self.adapter = None;
        self.binding = None;
        self.lifecycle_ready = false;
        self.generation = self.generation.wrapping_add(1);
        let result = match application::setup_discover(transport, store, record, account, now) {
            Ok(result) => result,
            Err(error) => {
                self.discovered = None;
                self.setup = SetupStatus::SelectionNeeded;
                return Err(error);
            }
        };
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
    pub fn accept<P: SelectionStore>(
        &mut self,
        store: &mut P,
        device_id: &str,
        expected_generation: u64,
        sink_store: S,
        sink: H,
    ) -> Result<(), application::ApplicationError> {
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
        self.binding = Some(binding);
        self.adapter = Some(adapter);
        self.setup = SetupStatus::SubscriptionPending;
        self.discovered = None;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    /// Attach the adapter only after durable selection has been read back and account-bound.
    pub fn activate<P: SelectionStore>(
        &mut self,
        store: &mut P,
        account: u64,
        sink_store: S,
        sink: H,
    ) -> Result<(), application::ApplicationError> {
        self.adapter = None;
        self.binding = None;
        self.lifecycle_ready = false;
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
