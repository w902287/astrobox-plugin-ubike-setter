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
    /// Stable key binding this plugin install to phone-GPS uploads (/api/loc).
    #[serde(default)]
    pub loc_key: String,
    /// Android automation deep link (HTTP Shortcuts / Tasker / MacroDroid)
    /// that runs the GPS-upload task. iOS uses shortcuts:// with a fixed name.
    #[serde(default)]
    pub android_trigger_url: String,
}

pub struct AppState {
    pub config: Config,
    /// Host platform string from os.platform(): "android" / "ios" / ...
    pub platform: String,
    /// Last device-list refresh (ms since epoch) — throttle host calls.
    pub last_device_refresh_ms: u128,
    pub devices: Vec<(String, String)>,
    /// Place search results: (display_name, lat, lng)
    pub results: Vec<(String, f64, f64)>,
    /// One-line preview of nearby stations for the last picked place.
    pub preview: String,
    pub selected_scenario: usize,
    pub query: String,
    pub notice: Option<String>,
    /// Set after launching the GPS shortcut; on next UI render we auto-pull.
    pub awaiting_gps: bool,
    /// Last phone GPS fix shown on the nearby tab (lat, lng).
    pub last_fix: Option<(f64, f64)>,
    pub root_element: Option<String>,
    pub registered: bool,
}

static STATE: OnceLock<Mutex<AppState>> = OnceLock::new();

pub fn state() -> &'static Mutex<AppState> {
    STATE.get_or_init(|| {
        Mutex::new(AppState {
            config: Config::default(),
            platform: String::new(),
            last_device_refresh_ms: 0,
            devices: Vec::new(),
            results: Vec::new(),
            preview: String::new(),
            selected_scenario: 0,
            query: String::new(),
            notice: None,
            awaiting_gps: false,
            last_fix: None,
            root_element: None,
            registered: false,
        })
    })
}

pub fn load() {
    let platform = wit_bindgen::block_on(crate::astrobox::psys_host::os::platform().into_future());
    {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.platform = platform.to_lowercase();
    }
    let cfg = fs::read_to_string(CONFIG_PATH)
        .ok()
        .and_then(|s| serde_json::from_str::<Config>(&s).ok())
        .unwrap_or_default();
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.config = cfg;
    if st.config.loc_key.is_empty() {
        // pseudo-random 12-char key from time + addresses; good enough for a
        // per-install binding token.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let chars: Vec<char> = "abcdefghjkmnpqrstuvwxyz23456789".chars().collect();
        let mut key = String::new();
        let mut x = seed ^ 0x9e3779b97f4a7c15;
        for _ in 0..12 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            key.push(chars[(x as usize) % chars.len()]);
        }
        st.config.loc_key = key;
        drop(st);
        save();
    }
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

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Throttled refresh (min 1.5s apart) — safe to call on every UI render.
pub fn refresh_devices() {
    {
        let st = state().lock().unwrap_or_else(|p| p.into_inner());
        if now_ms().saturating_sub(st.last_device_refresh_ms) < 1500 {
            return;
        }
    }
    force_refresh_devices();
}

/// Async refresh for spawned tasks: awaits host futures instead of block_on
/// (block_on inside a spawned task deadlocks the single-threaded executor).
pub async fn refresh_devices_async() {
    let mut devices = device::get_connected_device_list().into_future().await;
    if devices.is_empty() {
        devices = device::get_device_list().into_future().await;
    }
    let mapped = devices
        .into_iter()
        .map(|d| (d.addr, d.name))
        .collect::<Vec<_>>();
    {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.devices = mapped;
        st.last_device_refresh_ms = now_ms();
    }
    ensure_registered_async().await;
}

pub async fn ensure_registered_async() {
    let addrs: Vec<String> = {
        let st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.devices.iter().map(|(a, _)| a.clone()).collect()
    };
    if addrs.is_empty() {
        let _ = register::register_interconnect_recv("", crate::QA_PKG)
            .into_future()
            .await;
    }
    for addr in addrs {
        let _ = register::register_interconnect_recv(&addr, crate::QA_PKG)
            .into_future()
            .await;
    }
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.registered = true;
}

pub async fn push_config_async(addr: &str) {
    let (msg, addrs) = build_push(addr);
    for a in addrs {
        let _ = interconnect::send_qaic_message(&a, crate::QA_PKG, &msg)
            .into_future()
            .await;
    }
}

fn build_push(addr: &str) -> (String, Vec<String>) {
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
    let msg = serde_json::json!({"tag": "cfg", "coords": coords_json}).to_string();
    (msg, addrs)
}

/// Unthrottled refresh — used on load and DeviceAction events.
pub fn force_refresh_devices() {
    let mut devices = wit_bindgen::block_on(device::get_connected_device_list().into_future());
    if devices.is_empty() {
        devices = wit_bindgen::block_on(device::get_device_list().into_future());
    }
    let mapped = devices
        .into_iter()
        .map(|d| (d.addr, d.name))
        .collect::<Vec<_>>();
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.devices = mapped;
    st.last_device_refresh_ms = now_ms();
    drop(st);
    ensure_registered();
}

/// Register interconnect-recv for our quick app on all devices (idempotent-ish).
pub fn ensure_registered() {
    let addrs: Vec<String> = {
        let st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.devices.iter().map(|(a, _)| a.clone()).collect()
    };
    if addrs.is_empty() {
        let _ = wit_bindgen::block_on(
            register::register_interconnect_recv("", crate::QA_PKG).into_future(),
        );
    }
    for addr in addrs {
        let _ = wit_bindgen::block_on(
            register::register_interconnect_recv(&addr, crate::QA_PKG).into_future(),
        );
    }
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.registered = true;
}

/// Push saved config (sync path — only call from sync event context).
pub fn push_config_to(addr: &str) {
    let (msg, addrs) = build_push(addr);
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
