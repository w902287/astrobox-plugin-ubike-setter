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
