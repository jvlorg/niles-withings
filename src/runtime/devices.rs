//! Per-account collection of independently scoped sleep-mat event adapters.
use crate::{
    adapter::{Adapter, Sink},
    application::{self, WebhookRequest, WebhookResponse},
    config::Identity,
    coordinator::{ScopedBackend, ScopedStore},
    withings_client::Device,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

const MAX_INVENTORY: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    InvalidInventory,
    Storage,
    Delivery,
}
impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RegistryError {}

impl fmt::Debug for ReconcileResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReconcileResult")
            .field("accepted", &self.accepted)
            .field("retired_ids", &"[REDACTED]")
            .finish()
    }
}

/// Metadata accepted from a successfully reconciled inventory, plus retired IDs.
#[derive(Clone, PartialEq, Eq)]
pub struct ReconcileResult {
    pub accepted: Vec<Device>,
    pub retired_ids: Vec<String>,
}

struct Runtime<B: ScopedBackend, H: Sink> {
    adapter: Adapter<ScopedStore<B>, H>,
    metadata: Device,
}

/// Owns delivery adapters only; account OAuth and lifecycle readiness remain with the caller.
pub struct DeviceRegistry<B: ScopedBackend, H: Sink> {
    account: Option<u64>,
    devices: BTreeMap<String, Runtime<B, H>>,
    delivery_enabled: bool,
    inventory_valid: bool,
}
impl<B: ScopedBackend, H: Sink> Default for DeviceRegistry<B, H> {
    fn default() -> Self {
        Self {
            account: None,
            devices: BTreeMap::new(),
            delivery_enabled: false,
            inventory_valid: false,
        }
    }
}
impl<B: ScopedBackend, H: Sink> DeviceRegistry<B, H> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn account(&self) -> Option<u64> {
        self.account
    }
    pub fn device_ids(&self) -> Vec<String> {
        self.devices.keys().cloned().collect()
    }
    /// Gate webhook delivery independently from inventory ownership/readiness.
    pub fn set_delivery_enabled(&mut self, enabled: bool) {
        self.delivery_enabled = enabled;
    }

    /// Switch account before setup: all old adapters are revoked even if subsequent
    /// inventory validation or storage loading fails.
    pub fn begin_account(&mut self, account: u64) {
        if self.account != Some(account) {
            self.revoke_all();
            self.account = (account != 0).then_some(account);
        }
    }
    pub fn revoke_all(&mut self) {
        self.devices.clear();
        self.delivery_enabled = false;
        self.inventory_valid = false;
        self.account = None;
    }

    /// Validate the complete inventory before touching existing runtimes. Adapter
    /// construction loads durable state into a temporary map; any failure drops
    /// that map and leaves prior same-account runtimes unchanged.
    pub fn reconcile_inventory<BF, SF>(
        &mut self,
        account: u64,
        inventory: &[Device],
        mut backend_factory: BF,
        mut sink_factory: SF,
    ) -> Result<ReconcileResult, RegistryError>
    where
        BF: FnMut(&Device) -> B,
        SF: FnMut(&Device) -> H,
    {
        if self.account != Some(account) {
            self.revoke_all();
            self.account = (account != 0).then_some(account);
        }
        self.inventory_valid = false;
        if account == 0 || inventory.len() > MAX_INVENTORY {
            return Err(RegistryError::InvalidInventory);
        }
        let mut rows = BTreeMap::<String, Device>::new();
        let mut conflicting = BTreeSet::<String>::new();
        for device in inventory {
            if !valid_device(device) {
                return Err(RegistryError::InvalidInventory);
            }
            if let Some(prior) = rows.insert(device.device_id.clone(), device.clone())
                && prior != *device
            {
                conflicting.insert(device.device_id.clone());
            }
        }
        if !conflicting.is_empty() {
            return Err(RegistryError::InvalidInventory);
        }
        let eligible: BTreeMap<_, _> = rows
            .into_iter()
            .filter(|(_, d)| d.device_type == "Sleep Monitor")
            .collect();
        let accepted: Vec<_> = eligible.values().cloned().collect();
        let retained: BTreeSet<_> = self
            .devices
            .keys()
            .filter(|id| {
                eligible
                    .get(*id)
                    .is_some_and(|device| self.devices[*id].metadata == *device)
            })
            .cloned()
            .collect();
        let mut staged = BTreeMap::new();
        for (id, device) in &eligible {
            if retained.contains(id) {
                continue;
            }
            let identity = Identity::new(account.to_string(), id.clone())
                .map_err(|_| RegistryError::InvalidInventory)?;
            let store = ScopedStore::new(backend_factory(device), &account.to_string(), id)
                .map_err(|_| RegistryError::InvalidInventory)?;
            let adapter = Adapter::new(identity, store, sink_factory(device)).map_err(|error| {
                if matches!(error, crate::adapter::DeliveryError::Storage) {
                    RegistryError::Storage
                } else {
                    RegistryError::Delivery
                }
            })?;
            staged.insert(
                id.clone(),
                Runtime {
                    adapter,
                    metadata: device.clone(),
                },
            );
        }
        let retired_ids = self
            .devices
            .keys()
            .filter(|id| !eligible.contains_key(*id))
            .cloned()
            .collect();
        self.devices.retain(|id, _| retained.contains(id));
        self.devices.extend(staged);
        self.inventory_valid = true;
        Ok(ReconcileResult {
            accepted,
            retired_ids,
        })
    }

    /// Route through the established bounded parser only to an owned identity.
    pub fn webhook(
        &mut self,
        request: &WebhookRequest,
        now: i64,
        max_age: i64,
        skew: i64,
    ) -> WebhookResponse {
        if !self.delivery_enabled || !self.inventory_valid {
            return unavailable();
        }
        if request.method != "POST"
            || request.content_type.split(';').next().map(str::trim)
                != Some("application/x-www-form-urlencoded")
        {
            return unavailable();
        }
        // Parse once against each owned identity; only the matching identity can
        // validate. The adapter repeats the same canonical parser before applying.
        for runtime in self.devices.values_mut() {
            // Exact identity validation through the existing bounded parser occurs
            // inside Adapter::deliver; parser errors simply fall through to the
            // next owned runtime, without mutating any other adapter.
            let identity = runtime.adapter.identity();
            if crate::provider::parse(&request.body, identity, now, max_age, skew).is_ok() {
                return application::on_webhook(&mut runtime.adapter, request, now, max_age, skew);
            }
        }
        unavailable()
    }
}
fn valid_device(d: &Device) -> bool {
    !d.device_id.is_empty()
        && d.device_id.len() <= 128
        && !d.device_id.bytes().any(|b| b.is_ascii_control())
        && !d.device_type.is_empty()
        && d.device_type.len() <= 128
        && !d.model.is_empty()
        && d.model.len() <= 128
        && d.model_id != 0
}
fn unavailable() -> WebhookResponse {
    WebhookResponse {
        status: 503,
        body: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::Reading;
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    #[derive(Default)]
    struct BackendData {
        values: HashMap<String, String>,
        fail_load: bool,
    }
    #[derive(Clone, Default)]
    struct Backend(Rc<RefCell<BackendData>>);
    impl ScopedBackend for Backend {
        type Error = &'static str;
        fn load_value(&mut self, key: &str) -> Result<Option<String>, Self::Error> {
            let data = self.0.borrow();
            if data.fail_load {
                Err("load failure")
            } else {
                Ok(data.values.get(key).cloned())
            }
        }
        fn save_value(&mut self, key: &str, value: &str) -> Result<(), Self::Error> {
            self.0
                .borrow_mut()
                .values
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }
    }
    #[derive(Clone, Default)]
    struct Output(Rc<RefCell<Vec<&'static str>>>);
    impl Sink for Output {
        type Error = &'static str;
        fn publish_reading(&mut self, _: Reading) -> Result<(), Self::Error> {
            Ok(())
        }
        fn publish_action(&mut self, action: &'static str) -> Result<(), Self::Error> {
            self.0.borrow_mut().push(action);
            Ok(())
        }
    }
    fn device(id: &str) -> Device {
        Device {
            device_id: id.into(),
            device_type: "Sleep Monitor".into(),
            model: "Aura".into(),
            model_id: 1,
        }
    }
    fn request(id: &str, action: &str, timestamp: i64) -> WebhookRequest {
        WebhookRequest {
            method: "POST".into(),
            content_type: "application/x-www-form-urlencoded".into(),
            body: format!(
                "userid={}&deviceid={id}&date={timestamp}&appli={}",
                if id == "mat-a" && timestamp == 101 && action == "bed_in" {
                    8
                } else {
                    7
                },
                if action == "bed_in" { 50 } else { 51 }
            )
            .into_bytes(),
        }
    }
    fn reconcile(
        registry: &mut DeviceRegistry<Backend, Output>,
        account: u64,
        inventory: &[Device],
        data: &Backend,
        outputs: &HashMap<String, Output>,
    ) -> Result<ReconcileResult, RegistryError> {
        registry.reconcile_inventory(
            account,
            inventory,
            |_| Backend(Rc::clone(&data.0)),
            |d| outputs.get(&d.device_id).unwrap().clone(),
        )
    }

    #[test]
    fn devices_isolate_replay_restart_account_and_removal() {
        let backend = Backend::default();
        let a = Output::default();
        let b = Output::default();
        let outputs = HashMap::from([("mat-a".into(), a.clone()), ("mat-b".into(), b.clone())]);
        let mut registry = DeviceRegistry::new();
        reconcile(
            &mut registry,
            7,
            &[device("mat-a"), device("mat-b")],
            &backend,
            &outputs,
        )
        .unwrap();
        registry.set_delivery_enabled(true);
        assert_eq!(
            registry
                .webhook(&request("mat-a", "bed_in", 100), 100, 1000, 10)
                .status,
            200
        );
        assert_eq!(
            registry
                .webhook(&request("mat-b", "bed_in", 100), 100, 1000, 10)
                .status,
            200
        );
        assert_eq!(
            registry
                .webhook(&request("mat-a", "bed_in", 100), 100, 1000, 10)
                .status,
            200
        );
        assert_eq!(&*a.0.borrow(), &["bed_in"]);
        assert_eq!(&*b.0.borrow(), &["bed_in"]);

        let mut recreated = DeviceRegistry::new();
        reconcile(
            &mut recreated,
            7,
            &[device("mat-a"), device("mat-b")],
            &backend,
            &outputs,
        )
        .unwrap();
        recreated.set_delivery_enabled(true);
        assert_eq!(
            recreated
                .webhook(&request("mat-a", "bed_out", 99), 100, 1000, 10)
                .status,
            200
        );
        assert_eq!(&*a.0.borrow(), &["bed_in"]);
        assert_eq!(
            recreated
                .webhook(&request("mat-a", "bed_out", 101), 101, 1000, 10)
                .status,
            200
        );
        assert_eq!(&*a.0.borrow(), &["bed_in", "bed_out"]);

        let same_id = HashMap::from([("mat-a".into(), Output::default())]);
        let mut other = DeviceRegistry::new();
        reconcile(&mut other, 8, &[device("mat-a")], &backend, &same_id).unwrap();
        other.set_delivery_enabled(true);
        assert_eq!(
            other
                .webhook(&request("mat-a", "bed_in", 101), 101, 1000, 10)
                .status,
            200
        );
        assert_eq!(same_id["mat-a"].0.borrow().len(), 1);

        let retired = reconcile(&mut registry, 7, &[device("mat-a")], &backend, &outputs).unwrap();
        assert_eq!(retired.retired_ids, vec!["mat-b"]);
        assert_eq!(
            registry
                .webhook(&request("mat-b", "bed_out", 102), 102, 1000, 10)
                .status,
            503
        );
    }

    #[test]
    fn invalid_inventory_conflict_and_storage_failure_are_atomic() {
        let backend = Backend::default();
        let out = Output::default();
        let outputs = HashMap::from([("mat-a".into(), out.clone()), ("mat-b".into(), out.clone())]);
        let mut registry = DeviceRegistry::new();
        reconcile(&mut registry, 7, &[device("mat-a")], &backend, &outputs).unwrap();
        registry.set_delivery_enabled(true);
        assert_eq!(
            reconcile(
                &mut registry,
                7,
                &[device("mat-a"), device("mat-a")],
                &backend,
                &outputs
            )
            .unwrap()
            .accepted
            .len(),
            1
        );
        let mut conflict = device("mat-a");
        conflict.model = "Other".into();
        assert_eq!(
            reconcile(
                &mut registry,
                7,
                &[device("mat-a"), conflict],
                &backend,
                &outputs
            ),
            Err(RegistryError::InvalidInventory)
        );
        assert_eq!(
            registry
                .webhook(&request("mat-a", "bed_in", 100), 100, 1000, 10)
                .status,
            503
        );
        registry.set_delivery_enabled(true);
        assert_eq!(
            registry
                .webhook(&request("mat-a", "bed_in", 100), 100, 1000, 10)
                .status,
            503
        );

        let broken = Backend(Rc::clone(&backend.0));
        broken.0.borrow_mut().fail_load = true;
        assert_eq!(
            registry.reconcile_inventory(
                7,
                &[device("mat-a"), device("mat-b")],
                |_| broken.clone(),
                |_| out.clone()
            ),
            Err(RegistryError::Storage)
        );
        assert_eq!(registry.device_ids(), vec!["mat-a"]);
        assert_eq!(
            registry
                .webhook(&request("mat-a", "bed_out", 101), 101, 1000, 10)
                .status,
            503
        );
        broken.0.borrow_mut().fail_load = false;
        reconcile(
            &mut registry,
            7,
            &[device("mat-a"), device("mat-b")],
            &backend,
            &outputs,
        )
        .unwrap();
        assert_eq!(
            registry
                .webhook(&request("mat-a", "bed_in", 102), 102, 1000, 10)
                .status,
            200
        );

        registry.begin_account(8);
        assert!(registry.device_ids().is_empty());
        assert_eq!(
            registry
                .webhook(&request("mat-a", "bed_out", 102), 102, 1000, 10)
                .status,
            503
        );
    }
}
