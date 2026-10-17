//! Compact host-neutral owner for credentials, subscription lifecycle and event delivery.
pub mod devices;
use crate::{
    adapter::Sink,
    application::{
        self, AccountSecretStore, CallbackResult, SelectionStore, WebhookRequest, WebhookResponse,
    },
    coordinator::{Coordinator, ScopedBackend, ScopedStore, SetupView},
    lifecycle::{self, Clock, Lifecycle, LifecycleStatus, SecureTokenStore},
    withings_client::{self, CallbackUrl, HttpTransport, Secret},
};

pub trait AccountRecordRead: AccountSecretStore {
    fn read_account_record(&mut self) -> Result<Option<String>, Self::Error>;
}

// Coordinator's backend type is host-specific, so the runtime retains it as a generic.
pub struct Owner<B: ScopedBackend, H: Sink> {
    lifecycle: Lifecycle,
    coordinator: Coordinator<ScopedStore<B>, H>,
    registry: devices::DeviceRegistry<B, H>,
    automatic: bool,
    record: Option<application::SecureAccountRecord>,
}

impl<B: ScopedBackend, H: Sink> Owner<B, H> {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            lifecycle: Lifecycle::new(0, None, None),
            coordinator: Coordinator::default(),
            registry: devices::DeviceRegistry::new(),
            automatic: false,
            record: None,
        }
    }

    pub fn restore<R: AccountRecordRead>(
        &mut self,
        store: &mut R,
        account: u64,
        now: i64,
    ) -> Result<(), application::ApplicationError> {
        self.record = None;
        self.registry.revoke_all();
        self.automatic = false;
        self.lifecycle = Lifecycle::new(0, None, None);
        // Deliberately revoke before reading; missing bytes are expected here.
        let _ = self.coordinator.restore_account(None, account, now);
        let raw = store
            .read_account_record()
            .map_err(|_| application::ApplicationError::Storage)?;
        let raw = raw.ok_or(application::ApplicationError::InvalidRecord)?;
        let (tokens, metadata) = self.coordinator.restore_account(Some(&raw), account, now)?;
        self.record = Some(application::SecureAccountRecord::from_tokens(
            &tokens, metadata,
        )?);
        self.lifecycle = Lifecycle::new(account, Some(tokens), Some(metadata));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn callback<T: HttpTransport, R: AccountRecordRead>(
        &mut self,
        transport: &mut T,
        store: &mut R,
        callback: &CallbackUrl,
        query: &[(String, String)],
        person: Option<&str>,
        client_id: &str,
        client_secret: &Secret,
        now: i64,
    ) -> Result<CallbackResult, application::ApplicationError> {
        self.record = None;
        self.registry.revoke_all();
        self.automatic = false;
        self.lifecycle = Lifecycle::new(0, None, None);
        self.coordinator
            .set_lifecycle(lifecycle::Status::AuthorizationRequired);
        let result = application::on_callback(
            transport,
            store,
            callback,
            query,
            person,
            client_id,
            client_secret,
            now,
        )?;
        if let CallbackResult::Stored { user_id } = result {
            self.restore(store, user_id, now)?;
            Ok(CallbackResult::Stored { user_id })
        } else {
            Ok(result)
        }
    }

    pub fn discover<T: HttpTransport, P: SelectionStore>(
        &mut self,
        t: &mut T,
        p: &mut P,
        now: i64,
    ) -> Result<crate::coordinator::SetupStatus, application::ApplicationError> {
        let record = self
            .record
            .as_ref()
            .ok_or(application::ApplicationError::RecoveryRequired)?;
        self.coordinator.discover(t, p, record, now)
    }
    pub fn accept<P: SelectionStore>(
        &mut self,
        p: &mut P,
        id: &str,
        generation: u64,
        backend: B,
        sink: H,
    ) -> Result<(), application::ApplicationError> {
        self.coordinator
            .accept_scoped(p, id, generation, backend, sink)
    }
    /// Begin choosing a replacement mat without changing durable credentials,
    /// selection, or presence history. Delivery is revoked until acceptance.
    pub fn switch_mat(&mut self) {
        self.registry.revoke_all();
        self.automatic = false;
        self.coordinator.switch_device();
    }
    /// Record a failed interactive discovery attempt and revoke the active
    /// selection without modifying its durable selection or presence history.
    pub fn discovery_failed(&mut self) {
        self.registry.revoke_all();
        self.automatic = false;
        self.coordinator.discovery_failed();
    }
    /// Pause this in-memory owner. No provider unsubscribe or stored-secret
    /// deletion is performed; callers may restore validated credentials later.
    pub fn pause(&mut self) {
        self.record = None;
        self.registry.revoke_all();
        self.automatic = false;
        self.lifecycle = Lifecycle::new(0, None, None);
        self.coordinator.begin_account(None);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile<T: HttpTransport, R: SecureTokenStore, C: Clock>(
        &mut self,
        t: &mut T,
        s: &mut R,
        c: &C,
        client_id: &str,
        secret: &Secret,
        callback: &CallbackUrl,
        comment: &str,
    ) -> LifecycleStatus {
        let mut status = self
            .lifecycle
            .reconcile_at(t, s, c, client_id, secret, callback, comment);
        self.coordinator.set_lifecycle(status.status);
        if status.status != lifecycle::Status::Ready {
            self.registry.set_delivery_enabled(false);
        }
        if matches!(
            status.status,
            lifecycle::Status::Ready | lifecycle::Status::SubscriptionsIncomplete
        ) {
            self.record = self
                .lifecycle
                .credentials()
                .and_then(|(t, m)| application::SecureAccountRecord::from_tokens(&t, m).ok());
            if self.record.is_none() {
                status = LifecycleStatus {
                    status: lifecycle::Status::AuthorizationRequired,
                    reason: Some(lifecycle::BlockReason::UnknownOrInvalidMetadata),
                    subscriptions: [false; 2],
                };
                self.lifecycle = Lifecycle::new(0, None, None);
                self.registry.revoke_all();
                self.automatic = false;
                self.coordinator
                    .set_lifecycle(lifecycle::Status::AuthorizationRequired);
            }
        } else {
            self.record = None;
            self.registry.revoke_all();
            self.automatic = false;
        }
        if self.automatic {
            self.registry
                .set_delivery_enabled(status.status == lifecycle::Status::Ready);
        }
        status
    }
    pub fn webhook(
        &mut self,
        request: &WebhookRequest,
        now: i64,
        max_age: i64,
        skew: i64,
    ) -> WebhookResponse {
        if self.automatic {
            self.registry.webhook(request, now, max_age, skew)
        } else {
            self.coordinator.webhook(request, now, max_age, skew)
        }
    }

    pub fn discover_automatic<T: HttpTransport, BF, SF>(
        &mut self,
        transport: &mut T,
        now: i64,
        backend_factory: BF,
        sink_factory: SF,
    ) -> Result<devices::ReconcileResult, application::ApplicationError>
    where
        BF: FnMut(&withings_client::Device) -> B,
        SF: FnMut(&withings_client::Device) -> H,
    {
        self.automatic = true;
        self.registry.set_delivery_enabled(false);
        let record = self
            .record
            .as_ref()
            .ok_or(application::ApplicationError::RecoveryRequired)?;
        if !record.usable_at(now) {
            self.registry.revoke_all();
            self.automatic = true;
            return Err(application::ApplicationError::RecoveryRequired);
        }
        let token = record.access_token()?;
        let inventory = match withings_client::list_devices(transport, &token) {
            Ok(devices) => devices,
            Err(_) => {
                self.registry.revoke_all();
                self.automatic = true;
                return Err(application::ApplicationError::Provider);
            }
        };
        let result = self
            .registry
            .reconcile_inventory(record.user_id(), &inventory, backend_factory, sink_factory)
            .map_err(|_| application::ApplicationError::InvalidRecord)?;
        self.automatic = true;
        self.registry
            .set_delivery_enabled(self.lifecycle.status(now).status == lifecycle::Status::Ready);
        Ok(result)
    }
    pub fn generation(&self) -> u64 {
        self.coordinator.generation()
    }
    pub fn activate<P: SelectionStore>(
        &mut self,
        p: &mut P,
        account: u64,
        backend: B,
        sink: H,
    ) -> Result<(), application::ApplicationError> {
        self.coordinator.activate_scoped(p, account, backend, sink)
    }
    pub fn presentation(&self) -> SetupView {
        self.coordinator.presentation()
    }
}
