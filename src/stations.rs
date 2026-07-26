//! YouBike station search via the official open-data endpoint (Taipei 2.0).

use waki::Client;

/// Fetch station list and filter by keyword. Returns up to `limit` matches:
/// (name, lat, lng, area).
pub fn search(keyword: &str, limit: usize) -> Result<Vec<(String, f64, f64, String)>, String> {
    let url = "https://tcgbusfs.blob.core.windows.net/dotapp/youbike/v2/youbike_immediate.json";
    let resp = Client::new()
        .get(url)
        .connect_timeout(std::time::Duration::from_secs(15))
        .send()
        .map_err(|e| format!("request failed: {e}"))?;
    if resp.status_code() != 200 {
        return Err(format!("HTTP {}", resp.status_code()));
    }
    let body = resp.body().map_err(|e| format!("read failed: {e}"))?;
    let list: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("json failed: {e}"))?;
    let arr = list.as_array().ok_or("unexpected shape")?;

    let kw = keyword.trim().to_lowercase();
    let mut out = Vec::new();
    for s in arr {
        let name_raw = s.get("sna").and_then(|v| v.as_str()).unwrap_or("");
        let name = name_raw
            .replace("YouBike2.0_", "")
            .replace("YouBike1.0_", "");
        let area = s.get("sarea").and_then(|v| v.as_str()).unwrap_or("");
        if !kw.is_empty() {
            let hay = format!("{} {}", name.to_lowercase(), area.to_lowercase());
            if !hay.contains(&kw) {
                continue;
            }
        }
        let lat = s
            .get("latitude")
            .and_then(value_to_f64)
            .unwrap_or_default();
        let lng = s
            .get("longitude")
            .and_then(value_to_f64)
            .unwrap_or_default();
        if lat == 0.0 || lng == 0.0 {
            continue;
        }
        out.push((name, lat, lng, area.to_string()));
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn value_to_f64(v: &serde_json::Value) -> Option<f64> {
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    v.as_str().and_then(|s| s.parse::<f64>().ok())
}

/// Band asked for live stations for a scenario: compute nearest 4 from the
/// saved coord and reply over interconnect with the same shape the Worker
/// used, so the quick app can render it unchanged.
pub fn reply_stations(addr: &str, scenario: usize) {
    fn send_json(addr: &str, value: serde_json::Value) {
        use crate::astrobox::psys_host::interconnect;
        let text = value.to_string();
        let _ = wit_bindgen::block_on(
            interconnect::send_qaic_message(addr, crate::QA_PKG, &text).into_future(),
        );
    }

    let coord = {
        let st = crate::state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.config.coords[scenario].clone()
    };
    let Some(coord) = coord else {
        send_json(addr, serde_json::json!({
            "tag": "stations",
            "scenario": scenario,
            "error": "not-configured",
        }));
        return;
    };

    let url = "https://tcgbusfs.blob.core.windows.net/dotapp/youbike/v2/youbike_immediate.json";
    let resp = Client::new()
        .get(url)
        .connect_timeout(std::time::Duration::from_secs(15))
        .send();
    let body = match resp {
        Ok(r) if r.status_code() == 200 => r.body().unwrap_or_default(),
        Ok(r) => {
            send_json(addr, serde_json::json!({"tag":"stations","scenario":scenario,"error":format!("HTTP {}", r.status_code())}));
            return;
        }
        Err(e) => {
            send_json(addr, serde_json::json!({"tag":"stations","scenario":scenario,"error":format!("{e}")}));
            return;
        }
    };
    let Ok(list) = serde_json::from_slice::<serde_json::Value>(&body) else {
        send_json(addr, serde_json::json!({"tag":"stations","scenario":scenario,"error":"bad-json"}));
        return;
    };
    let Some(arr) = list.as_array() else {
        send_json(addr, serde_json::json!({"tag":"stations","scenario":scenario,"error":"bad-shape"}));
        return;
    };

    let mut rows: Vec<(f64, String, i64, i64)> = Vec::new();
    for s in arr {
        let lat = s.get("latitude").and_then(value_to_f64).unwrap_or_default();
        let lng = s.get("longitude").and_then(value_to_f64).unwrap_or_default();
        if lat == 0.0 || lng == 0.0 {
            continue;
        }
        let name = s
            .get("sna")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .replace("YouBike2.0_", "")
            .replace("YouBike1.0_", "");
        let bikes = s
            .get("available_rent_bikes")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|x| x.parse().ok())))
            .unwrap_or(0);
        let docks = s
            .get("available_return_bikes")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|x| x.parse().ok())))
            .unwrap_or(0);
        let d = haversine_km(coord.lat, coord.lng, lat, lng);
        rows.push((d, name, bikes, docks));
    }
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let stations: Vec<serde_json::Value> = rows
        .iter()
        .take(4)
        .map(|(d, name, bikes, docks)| {
            serde_json::json!({
                "name": name,
                "bikes": bikes,
                "docks": docks,
                "dist": (d * 1000.0).round() as i64,
            })
        })
        .collect();
    send_json(
        addr,
        serde_json::json!({
            "tag": "stations",
            "scenario": scenario,
            "stations": stations,
            "source": crate::state::SCENARIO_NAMES[scenario],
        }),
    );
}

fn haversine_km(a_lat: f64, a_lng: f64, b_lat: f64, b_lng: f64) -> f64 {
    let r = 6371.0_f64;
    let dx = (b_lat - a_lat).to_radians();
    let dy = (b_lng - a_lng).to_radians();
    let h = (dx / 2.0).sin().powi(2)
        + a_lat.to_radians().cos() * b_lat.to_radians().cos() * (dy / 2.0).sin().powi(2);
    r * 2.0 * h.sqrt().atan2((1.0 - h).sqrt())
}
