//! Global state: three saved locations + device list + persistence.

use std::fs;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::astrobox::psys_host::{device, interconnect, register};

pub const SCENARIO_NAMES: [&str; 3] = ["住家", "公司", "車站"];
const CONFIG_PATH: &str = "/data/ubike_setter_config.json";

#[derive(Clone, Serialize, Deserialize)]
pub struct Coord {
    pub lat: f64,
    pub lng: f64,
    #[serde(default)]
    pub label: String,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub coords: [Option<Coord>; 3],
}

pub struct AppState {
    pub config: Config,
    pub devices: Vec<(String, String)>,
    /// Search results: (name, lat, lng, area)
    pub results: Vec<(String, f64, f64, String)>,
    pub selected_scenario: usize,
    pub query: String,
    pub notice: Option<String>,
    pub root_element: Option<String>,
    pub registered: bool,
}

static STATE: OnceLock<Mutex<AppState>> = OnceLock::new();

pub fn state() -> &'static Mutex<AppState> {
    STATE.get_or_init(|| {
        Mutex::new(AppState {
            config: Config::default(),
            devices: Vec::new(),
            results: Vec::new(),
            selected_scenario: 0,
            query: String::new(),
            notice: None,
            root_element: None,
            registered: false,
        })
    })
}

pub fn load() {
    let cfg = fs::read_to_string(CONFIG_PATH)
        .ok()
        .and_then(|s| serde_json::from_str::<Config>(&s).ok())
        .unwrap_or_default();
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.config = cfg;
}

pub fn save() {
    let cfg = {
        let st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.config.clone()
    };
    if let Ok(text) = serde_json::to_string(&cfg) {
        if let Err(err) = fs::write(CONFIG_PATH, text) {
            tracing::warn!("persist failed: {err}");
        }
    }
}

pub fn refresh_devices() {
    let devices = wit_bindgen::block_on(device::get_connected_device_list().into_future());
    let mapped = devices
        .into_iter()
        .map(|d| (d.addr, d.name))
        .collect::<Vec<_>>();
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.devices = mapped;
    drop(st);
    ensure_registered();
}

/// Register interconnect-recv for our quick app on all devices (idempotent-ish).
pub fn ensure_registered() {
    let addrs: Vec<String> = {
        let st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.devices.iter().map(|(a, _)| a.clone()).collect()
    };
    for addr in addrs {
        let _ = wit_bindgen::block_on(
            register::register_interconnect_recv(&addr, crate::QA_PKG).into_future(),
        );
    }
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.registered = true;
}

/// Push saved config to one device (or all if addr empty).
pub fn push_config_to(addr: &str) {
    let (cfg, addrs) = {
        let st = state().lock().unwrap_or_else(|p| p.into_inner());
        let addrs = if addr.is_empty() {
            st.devices.iter().map(|(a, _)| a.clone()).collect()
        } else {
            vec![addr.to_string()]
        };
        (st.config.clone(), addrs)
    };
    let coords_json: Vec<serde_json::Value> = cfg
        .coords
        .iter()
        .map(|c| match c {
            Some(c) => serde_json::json!({"lat": c.lat, "lng": c.lng, "label": c.label}),
            None => serde_json::Value::Null,
        })
        .collect();
    let msg = serde_json::json!({
        "tag": "cfg",
        "coords": coords_json,
    })
    .to_string();
    for a in addrs {
        let r = wit_bindgen::block_on(
            interconnect::send_qaic_message(&a, crate::QA_PKG, &msg).into_future(),
        );
        tracing::info!("push cfg to {a}: {:?}", r.is_ok());
    }
}

pub fn set_notice(text: impl Into<String>) {
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.notice = Some(text.into());
}
