//! Host-facing ownership for automatically discovered mats.
use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Fleet {
    pub account: Option<u64>,
    pub sinks: BTreeMap<String, DeviceSink>,
    pub error: Option<String>,
    pub ready: bool,
    pub inventory_valid: bool,
    pub subscriptions_ready: bool,
}
thread_local! { pub(super) static FLEET: RefCell<Fleet> = RefCell::new(Fleet::default()); }

pub(super) fn revoke() -> Result<(), String> {
    FLEET.with(|f| {
        let mut fleet = f.borrow_mut();
        fleet.ready = false;
        let mut failed = false;
        for sink in fleet.sinks.values() {
            *sink.occupancy.borrow_mut() = None;
            if let Ok(id) = sink.target() {
                failed |= devices::set_available(&id, false).is_err();
            }
        }
        if failed {
            Err("Could not revoke mat availability".into())
        } else {
            Ok(())
        }
    })
}
fn remembered_ids() -> Result<Vec<String>, String> {
    let ids: Vec<String> = match HostStore::get("host_devices")? {
        Some(raw) if raw.len() <= 65536 => {
            serde_json::from_str(&raw).map_err(|_| "Saved host device inventory is corrupt")?
        }
        Some(_) => return Err("Saved host device inventory is too large".into()),
        None => Vec::new(),
    };
    if ids.len() > 128
        || ids.iter().any(|id| {
            id.len() > 512 || !id.contains("/withings_") || id.chars().any(char::is_control)
        })
    {
        return Err("Saved host device inventory is invalid".into());
    }
    Ok(ids)
}
pub(super) fn reset_host() -> Result<(), String> {
    revoke()?;
    let mut ids = remembered_ids()?;
    if HostStore::get("selection")?.is_some() {
        let legacy = format!("{}/withings_sleep", setting("room")?);
        if !ids.contains(&legacy) {
            ids.push(legacy);
        }
    }
    for id in ids {
        register(&id)?;
        devices::set_available(&id, false).map_err(|_| "Could not revoke stored mat")?;
    }
    FLEET.with(|f| *f.borrow_mut() = Fleet::default());
    Ok(())
}
pub(super) fn register(id: &str) -> Result<(), String> {
    let mut ids = remembered_ids()?;
    if !ids.iter().any(|known| known == id) {
        if ids.len() >= 128 {
            return Err("Saved host device capacity reached".into());
        }
        ids.push(id.to_string());
        HostStore::set(
            "host_devices",
            &serde_json::to_string(&ids).map_err(|_| "Could not encode host inventory")?,
        )?;
    }
    devices::upsert(&types::Device {
        id: id.into(),
        class: types::DeviceClass::Sensor,
        light: types::LightCapabilities {
            color_temp: false,
            rgb: false,
        },
        controllable: false,
        state: types::DeviceState {
            on: None,
            brightness: None,
            color_temp_kelvin: None,
            rgb: None,
            temperature_celsius: None,
            humidity_percent: None,
            battery_percent: None,
            open: None,
            motion: None,
            illuminance_lux: None,
            position: None,
            readings: vec![],
        },
    })
    .map_err(|_| "Device registration failed".to_string())
}
fn stable_id(room: &str, account: u64, device: &str) -> String {
    // Exact UTF-8 hex is collision-free, unlike display labels or inventory order.
    let encoded: String = device
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{room}/withings_{account}_{encoded}")
}

pub(super) fn discover(
    owner: &mut Owner<HostStore, DeviceSink>,
    account: u64,
) -> Result<(), String> {
    let result = discover_inner(owner, account);
    if let Err(error) = &result {
        owner.discovery_failed();
        let _ = revoke();
        FLEET.with(|f| {
            let mut f = f.borrow_mut();
            f.inventory_valid = false;
            f.error = Some(error.clone());
        });
    }
    result
}

fn discover_inner(owner: &mut Owner<HostStore, DeviceSink>, account: u64) -> Result<(), String> {
    let room = setting("room")?;
    let legacy = HostStore::get("selection")?
        .map(|raw| niles_withings::application::Selection::restore(&raw))
        .transpose()
        .map_err(|_| "Saved device selection is corrupt".to_string())?
        .and_then(|selection| selection.binding_for(account).ok());
    let legacy_id = legacy.map(|binding| binding.identity());
    let changed = FLEET.with(|f| f.borrow().account != Some(account));
    if changed {
        revoke()?;
        FLEET.with(|f| {
            let mut f = f.borrow_mut();
            f.account = Some(account);
            f.sinks.clear();
        });
    }
    let mut registration_error = None;
    let result = owner.discover_automatic(
        &mut HostTransport,
        now()?,
        |_| HostStore,
        |device| {
            FLEET.with(|f| {
                let mut fleet = f.borrow_mut();
                if fleet
                    .sinks
                    .get(&device.device_id)
                    .is_some_and(|sink| sink.target().is_ok())
                {
                    return fleet.sinks[&device.device_id].clone();
                }
                let id = if legacy_id
                    .as_ref()
                    .is_some_and(|identity| identity.device_id == device.device_id)
                {
                    format!("{room}/withings_sleep")
                } else {
                    stable_id(&room, account, &device.device_id)
                };
                let sink = DeviceSink::default();
                match register(&id).and_then(|()| {
                    devices::set_available(&id, false)
                        .map_err(|_| "Could not initialize device availability".to_string())
                }) {
                    Ok(()) => sink.set_target(id),
                    Err(error) => registration_error = Some(error),
                }
                fleet.sinks.insert(device.device_id.clone(), sink.clone());
                sink
            })
        },
    );
    match result {
        Ok(inventory) if registration_error.is_none() => {
            FLEET.with(|f| -> Result<(), String> {
                let fleet = f.borrow();
                for (id, sink) in &fleet.sinks {
                    if !inventory.accepted.iter().any(|d| &d.device_id == id)
                        && let Ok(target) = sink.target()
                    {
                        devices::set_available(&target, false)
                            .map_err(|_| "Could not retire missing mat")?;
                    }
                }
                Ok(())
            })?;
            FLEET.with(|f| {
                let mut fleet = f.borrow_mut();
                fleet.inventory_valid = true;
                fleet.ready = fleet.subscriptions_ready;
                fleet
                    .sinks
                    .retain(|id, _| inventory.accepted.iter().any(|d| &d.device_id == id));
                fleet.error = None;
            });
            Ok(())
        }
        result => {
            FLEET.with(|f| f.borrow_mut().ready = false);
            owner.discovery_failed();
            let _ = revoke();
            let error = registration_error
                .unwrap_or_else(|| format!("Device discovery failed: {:?}", result.err()));
            FLEET.with(|f| f.borrow_mut().error = Some(error.clone()));
            Err(error)
        }
    }
}
