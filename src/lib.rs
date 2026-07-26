use wit_bindgen::FutureReader;

use crate::exports::astrobox::psys_plugin::{event_v3 as event, event_v3::EventType, lifecycle};

pub mod fetchbridge;
pub mod state;
pub mod stations;
pub mod ui;

wit_bindgen::generate!({
    path: "wit",
    world: "psys-world-v3",
    generate_all,
});

pub const QA_PKG: &str = "tw.youbike.band";

struct UbikeSetter;

fn immediate_string(value: String) -> FutureReader<String> {
    let (writer, reader) = wit_future::new::<String>(|| String::new());
    wit_bindgen::spawn(async move {
        let _ = writer.write(value).await;
    });
    reader
}

fn immediate_unit() -> FutureReader<()> {
    let (writer, reader) = wit_future::new::<()>(|| ());
    wit_bindgen::spawn(async move {
        let _ = writer.write(()).await;
    });
    reader
}

impl event::Guest for UbikeSetter {
    fn on_event(event_type: EventType, event_payload: String) -> FutureReader<String> {
        match event_type {
            EventType::InterconnectMessage => {
                handle_interconnect(&event_payload);
            }
            EventType::DeviceAction => {
                state::force_refresh_devices();
                ui::rerender();
            }
            _ => {}
        }
        immediate_string(String::new())
    }

    fn on_ui_event_v3(
        event_id: String,
        ev: crate::astrobox::psys_host::ui_v3::Event,
        event_payload: String,
    ) -> FutureReader<String> {
        ui::handle_ui_event(&event_id, ev, &event_payload);
        immediate_string(String::new())
    }

    fn on_ui_render(element_id: String) -> FutureReader<()> {
        // Paint immediately — the host shows "waiting for plugin render"
        // until the first render() call, so nothing slow may run before it.
        ui::render_root(&element_id);
        let (writer, reader) = wit_future::new::<()>(|| ());
        wit_bindgen::spawn(async move {
            state::refresh_devices();
            ui::auto_pull_gps_if_awaiting();
            ui::rerender();
            let _ = writer.write(()).await;
        });
        reader
    }

    fn on_card_render(_card_id: String) -> FutureReader<()> {
        immediate_unit()
    }
}

/// Handle messages from the quick app. We only care about a config pull:
/// `{"tag":"cfg-pull"}` -> reply with `{"tag":"cfg","coords":[...]}`.
fn handle_interconnect(payload: &str) {
    let parsed: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return,
    };
    let text = parsed
        .get("payloadText")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| payload.to_string());
    let addr = parsed
        .get("addr")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let inner: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return,
    };
    let tag = inner.get("tag").and_then(|v| v.as_str()).unwrap_or("");
    match tag {
        "__hs__" => {
            let count = inner.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            if count < 2 {
                let reply = serde_json::json!({"tag": "__hs__", "count": count + 1}).to_string();
                let _ = wit_bindgen::block_on(
                    crate::astrobox::psys_host::interconnect::send_qaic_message(
                        &addr,
                        QA_PKG,
                        &reply,
                    )
                    .into_future(),
                );
            }
        }
        "fetch" => {
            fetchbridge::handle_fetch(&addr, inner.clone());
        }
        "cfg-pull" => {
            tracing::info!("quick app requested config");
            state::push_config_to(&addr);
        }
        "stations-pull" => {
            let scenario = inner
                .get("scenario")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(3) as usize;
            if scenario == 3 {
                stations::reply_nearby_gps(&addr);
            } else {
                stations::reply_stations(&addr, scenario);
            }
        }
        _ => {}
    }
}

impl lifecycle::Guest for UbikeSetter {
    fn on_load() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .without_time()
            .try_init();
        state::load();
        wit_bindgen::spawn(async move {
            state::force_refresh_devices();
            tracing::info!("Ubike Setter: devices registered");
        });
        tracing::info!("Ubike Setter plugin loaded");
    }
}

export!(UbikeSetter);
