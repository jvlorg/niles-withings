//! Niles imports: encrypted account storage, scoped persistence, per-device publication and HTTP.
use super::*;
impl niles_withings::diagnostics::Backend for HostStore {
    type Error = String;
    fn get(&mut self, key: &str) -> Result<Option<String>, String> {
        Self::get(key)
    }
    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        Self::set(key, value)
    }
    fn delete(&mut self, key: &str) -> Result<(), String> {
        storage::delete(key);
        if storage::get(key).is_some() {
            Err("Diagnostic deletion failed".into())
        } else {
            Ok(())
        }
    }
    fn list(&mut self, prefix: &str) -> Result<Vec<String>, String> {
        Ok(storage::list_keys(prefix))
    }
}

#[derive(Clone)]
pub(super) struct HostStore;
impl HostStore {
    pub(super) fn get(key: &str) -> Result<Option<String>, String> {
        Ok(storage::get(key))
    }
    pub(super) fn set(key: &str, value: &str) -> Result<(), String> {
        if storage::set(key, value).is_ok() {
            return Ok(());
        }
        // Never reclaim evidence for a generic/transient storage failure. Confirm
        // the same key+value quota calculation used by the pinned Niles store.
        if !key.starts_with(niles_withings::diagnostics::PREFIX) {
            let others: usize = storage::list_keys("")
                .into_iter()
                .filter(|k| k != key)
                .filter_map(|k| storage::get(&k).map(|v| k.len().saturating_add(v.len())))
                .sum();
            if others.saturating_add(key.len()).saturating_add(value.len()) > 1_048_576 {
                let _ = niles_withings::diagnostics::reclaim(&mut HostStore);
                return storage::set(key, value)
                    .map_err(|_| "Host storage write failed".to_string());
            }
        }
        Err("Host storage write failed".to_string())
    }
}
impl AccountSecretStore for HostStore {
    type Error = String;
    fn set_account_record(&mut self, value: &str) -> Result<(), String> {
        secrets::set("account", value).map_err(|_| "Encrypted account write failed".into())
    }
}
impl AccountRecordRead for HostStore {
    fn read_account_record(&mut self) -> Result<Option<String>, String> {
        secrets::get("account")
            .ok_or_else(|| "Encrypted account record missing".to_string())
            .map(Some)
    }
}

impl SelectionStore for HostStore {
    type Error = String;
    fn load_selection(&mut self) -> Result<Option<String>, String> {
        Self::get("selection")
    }
    fn save_selection(&mut self, value: &str) -> Result<(), String> {
        Self::set("selection", value)
    }
}
impl ScopedBackend for HostStore {
    type Error = String;
    fn load_value(&mut self, key: &str) -> Result<Option<String>, String> {
        Self::get(key)
    }
    fn save_value(&mut self, key: &str, value: &str) -> Result<(), String> {
        Self::set(key, value)
    }
}
#[derive(Clone, Default)]
pub(super) struct DeviceSink {
    id: Rc<RefCell<Option<String>>>,
    pub(super) occupancy: Rc<RefCell<Option<bool>>>,
}
impl DeviceSink {
    pub(super) fn target(&self) -> Result<String, String> {
        self.id
            .borrow()
            .clone()
            .ok_or_else(|| "Registered Withings device is unavailable".into())
    }
    pub(super) fn set_target(&self, id: String) {
        *self.id.borrow_mut() = Some(id);
    }
}
impl Sink for DeviceSink {
    type Error = String;
    fn observe(
        &mut self,
        event: &niles_withings::provider::Event,
        observation: niles_withings::adapter::Observation,
    ) {
        super::delivery_diagnostics::observe(event, observation);
    }
    fn publish_reading(&mut self, reading: Reading) -> Result<(), String> {
        let id = self.target()?;
        *self.occupancy.borrow_mut() = None;
        let occupied = match reading {
            Reading::Occupied { occupied, .. } => Some(occupied),
            _ => None,
        };
        let result = match reading {
            Reading::Unknown { .. } => devices::set_available(&id, false)
                .map_err(|_| "Could not mark device unavailable".into()),
            Reading::Occupied {
                occupied,
                observed_at,
            } => devices::report_state(
                &id,
                &types::DeviceState {
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
                    readings: vec![
                        types::Reading {
                            name: "in_bed".into(),
                            value: if occupied { 1.0 } else { 0.0 },
                            unit: None,
                        },
                        types::Reading {
                            name: "observed_at".into(),
                            value: observed_at as f64,
                            unit: Some("unix_seconds".into()),
                        },
                    ],
                },
            )
            .map_err(|_| "Could not publish Withings reading".to_string())
            .and_then(|()| {
                devices::set_available(&id, true)
                    .map_err(|_| "Could not confirm availability".to_string())
            }),
        };
        if result.is_ok() {
            *self.occupancy.borrow_mut() = occupied;
        }
        result
    }
    fn publish_action(&mut self, action: &'static str) -> Result<(), String> {
        devices::report_action(&self.target()?, action)
            .map_err(|_| "Could not publish Withings action".into())
    }
}
pub(super) struct HostTransport;
impl HttpTransport for HostTransport {
    type Error = String;
    fn post(&mut self, r: HttpRequest) -> Result<HttpResponse, String> {
        let mut headers = vec![(
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        )];
        if let Some(t) = r.bearer {
            headers.push((
                "authorization".into(),
                format!("Bearer {}", t.expose_secret()),
            ));
        }
        let response = http::send(&http::Request {
            method: http::Method::Post,
            url: r.url.into(),
            headers,
            body: Some(r.form.into_bytes()),
            timeout_ms: Some(5000),
        })
        .map_err(|_| "Provider request failed".to_string())?;
        if response.body.len() <= 65536
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&response.body)
            && let Some(status) = value.get("status").and_then(|v| v.as_i64())
            && status != 0
        {
            // Only numeric protocol status: never payloads, URLs or credentials.
            eprintln!(
                "Withings request rejected: HTTP {}, provider status {}",
                response.status, status
            );
        }
        Ok(HttpResponse {
            status: response.status,
            body: response.body,
        })
    }
}
