#![allow(clippy::too_many_arguments)] // Generated WIT bindings expose protocol records.
wit_bindgen::generate!({
    world: "plugin",
    path: "wit/niles-integration.wit",
});
use niles::integration::types::{Device, DeviceClass, LightCapabilities};
use niles::integration::{
    config, devices,
    ui::{Node, NodeKind, TextBlock},
};
struct Plugin;
export!(Plugin);
fn empty_state() -> DeviceState {
    DeviceState {
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
        readings: Vec::new(),
    }
}
impl Guest for Plugin {
    fn init() -> Result<(), String> {
        let user = match config::get("user_id") {
            Some(config::SettingValue::Text(x)) => x,
            _ => return Err("Configure the Withings user id.".into()),
        };
        let device = match config::get("device_id") {
            Some(config::SettingValue::Text(x)) => x,
            _ => return Err("Configure the Sleep device id.".into()),
        };
        niles_withings::config::Identity::new(user, device).map_err(str::to_owned)?;
        let room = match config::get("room") {
            Some(config::SettingValue::Text(x)) if !x.is_empty() => x,
            _ => return Err("Choose the room containing the bed.".into()),
        };
        if !room
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err("Room must use lowercase letters, numbers, or underscores.".into());
        }
        devices::upsert(&Device {
            id: format!("{room}/presence"),
            class: DeviceClass::Sensor,
            light: LightCapabilities {
                color_temp: false,
                rgb: false,
            },
            controllable: false,
            state: empty_state(),
        })
        .map_err(|_| "Could not register bed sensor.".to_string())?;
        Ok(())
    }
    fn tools() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "status".into(),
            description: "Report whether Withings callback delivery is configured and whether a callback connection is active. This version does not receive callbacks, so it is always disconnected.".into(),
            schema: r#"{"type":"object","properties":{}}"#.into(),
        }]
    }
    fn call_tool(name: String, _: String) -> Result<String, String> {
        if name == "status" {
            Ok("Setup incomplete: callback transport is not available in this Niles interface; no Withings account is connected.".into())
        } else {
            Err("Unknown tool.".into())
        }
    }
    fn set_device(_: String, _: DeviceState) -> Result<(), String> {
        Err("Bed sensor is read-only.".into())
    }
    fn on_timer(_: String) {}
    fn on_event(_: Event) {}
    fn render(_: String, _: RenderContext) -> Result<UiTree, String> {
        let lines = [
            "Connection: disconnected",
            "Occupancy: unavailable while disconnected",
            "Last event: none",
        ];
        let mut nodes = vec![
            Node {
                kind: NodeKind::Stack,
                children: vec![1],
            },
            Node {
                kind: NodeKind::Card(Some("Withings Sleep".into())),
                children: (2..=4).collect(),
            },
        ];
        nodes.extend(lines.iter().map(|line| Node {
            kind: NodeKind::Text(TextBlock {
                value: (*line).into(),
                muted: false,
            }),
            children: vec![],
        }));
        Ok(UiTree { nodes })
    }
    fn on_action(
        _: String,
        _: String,
        _: ActionValue,
        _: RenderContext,
    ) -> Result<ActionResult, String> {
        Err("No Withings account is connected.".into())
    }
    fn on_mqtt_message(_: u32, _: String, _: Vec<u8>) {}
    fn on_mqtt_status(_: u32, _: ConnectionStatus) {}
    fn stop() {}
}
