//! Location search (geocoding) + nearby-station preview.
//!
//! The user searches a PLACE (e.g. "市政府"), picks it as the scenario's
//! center coordinate; the band then always shows the nearest 4 stations around
//! that point. Stations are never picked manually.

use std::sync::{Mutex, OnceLock};
use waki::Client;

struct BikeCache {
    at_ms: u128,
    rows: Vec<(f64, f64, String, i64, i64)>, // lat,lng,name,bikes,docks
}

static BIKE_CACHE: OnceLock<Mutex<Option<BikeCache>>> = OnceLock::new();

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Download + parse the station list, cached for 30 seconds.
fn all_stations() -> Result<Vec<(f64, f64, String, i64, i64)>, String> {
    let cache = BIKE_CACHE.get_or_init(|| Mutex::new(None));
    {
        let guard = cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = guard.as_ref() {
            if now_ms().saturating_sub(c.at_ms) < 30_000 {
                return Ok(c.rows.clone());
            }
        }
    }
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
    let mut rows = Vec::with_capacity(arr.len());
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
        rows.push((lat, lng, name, bikes, docks));
    }
    let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some(BikeCache { at_ms: now_ms(), rows: rows.clone() });
    Ok(rows)
}

/// Geocode a place name via Nominatim (same as the old web settings page).
/// Returns up to `limit` matches: (display_name, lat, lng).
pub fn geocode(keyword: &str, limit: usize) -> Result<Vec<(String, f64, f64)>, String> {
    let url = format!(
        "https://nominatim.openstreetmap.org/search?format=json&limit={}&accept-language=zh-TW&countrycodes=tw&q={}",
        limit,
        urlencode(keyword)
    );
    let resp = Client::new()
        .get(&url)
        .header("User-Agent", "ubike-setter-plugin/0.1 (astrobox)")
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
    let mut out = Vec::new();
    for item in arr {
        let name = item
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let lat = item.get("lat").and_then(value_to_f64).unwrap_or_default();
        let lng = item.get("lon").and_then(value_to_f64).unwrap_or_default();
        if lat == 0.0 || lng == 0.0 {
            continue;
        }
        let short = shorten_display_name(&name);
        out.push((short, lat, lng));
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

/// Nominatim display_name is very long ("台北市政府, 市府路, 信義區, ...").
/// Keep the first 3 comma parts for readability.
fn shorten_display_name(name: &str) -> String {
    let parts: Vec<&str> = name.split(',').map(str::trim).take(3).collect();
    parts.join("・")
}

/// Address-aware search. Full Taiwanese addresses (with floor/room suffixes)
/// often miss on Nominatim, so retry with progressively simplified forms:
/// full query -> cut after "號" (drop floor/room) -> drop house number (road
/// level). Returns (results, used_query_if_different_from_input).
pub fn geocode_smart(
    keyword: &str,
    limit: usize,
) -> Result<(Vec<(String, f64, f64)>, Option<String>), String> {
    let original = keyword.trim();
    let first = geocode(original, limit)?;
    if !first.is_empty() {
        return Ok((first, None));
    }
    for cand in simplify_candidates(original) {
        if cand == original {
            continue;
        }
        if let Ok(r) = geocode(&cand, limit) {
            if !r.is_empty() {
                return Ok((r, Some(cand)));
            }
        }
    }
    Ok((Vec::new(), None))
}

fn simplify_candidates(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(pos) = q.find('號') {
        // 1) keep up to 號: drops 樓/室/之X after the house number
        let cut = &q[..pos + '號'.len_utf8()];
        out.push(cut.to_string());
        // 2) drop the house number itself -> road level
        let road = cut
            .trim_end_matches('號')
            .trim_end_matches(|c: char| {
                c.is_ascii_digit()
                    || ('０'..='９').contains(&c)
                    || c == '之'
                    || c == '-'
                    || c == '－'
            })
            .trim_end_matches(['巷', '弄'])
            .trim_end_matches(|c: char| c.is_ascii_digit() || ('０'..='９').contains(&c));
        if !road.is_empty() && road != cut {
            out.push(road.to_string());
        }
    }
    out
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Read the real phone GPS position uploaded by an iOS Shortcut / Android
/// automation to the Worker: GET /api/loc?key=<device-bound key>.
/// Returns (lat, lng, age_seconds).
pub fn fetch_shortcut_location(key: &str) -> Result<(f64, f64, i64), String> {
    let url = format!(
        "https://youbike-band.w902287.workers.dev/api/loc?key={}",
        urlencode(key)
    );
    let resp = Client::new()
        .get(&url)
        .connect_timeout(std::time::Duration::from_secs(10))
        .send()
        .map_err(|e| format!("連線失敗: {e}"))?;
    if resp.status_code() == 404 {
        return Err("尚未收到手機定位，請先執行捷徑".to_string());
    }
    if resp.status_code() != 200 {
        return Err(format!("HTTP {}", resp.status_code()));
    }
    let body = resp.body().map_err(|e| format!("讀取失敗: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("格式錯誤: {e}"))?;
    let lat = v.get("lat").and_then(value_to_f64).unwrap_or_default();
    let lng = v.get("lng").and_then(value_to_f64).unwrap_or_default();
    let age = v.get("age_seconds").and_then(|x| x.as_i64()).unwrap_or(0);
    if lat == 0.0 || lng == 0.0 {
        return Err("座標為空".to_string());
    }
    Ok((lat, lng, age))
}

/// Nearest `count` stations around a coordinate: (name, bikes, docks, dist_m).
/// Backed by the 30-second station cache.
pub fn nearby(lat: f64, lng: f64, count: usize) -> Result<Vec<(String, i64, i64, i64)>, String> {
    let rows = all_stations()?;
    let mut scored: Vec<(f64, String, i64, i64)> = rows
        .into_iter()
        .map(|(slat, slng, name, bikes, docks)| {
            (haversine_km(lat, lng, slat, slng), name, bikes, docks)
        })
        .collect();
    scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored
        .into_iter()
        .take(count)
        .map(|(d, n, b, k)| (n, b, k, (d * 1000.0).round() as i64))
        .collect())
}

fn value_to_f64(v: &serde_json::Value) -> Option<f64> {
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    v.as_str().and_then(|s| s.parse::<f64>().ok())
}

/// Band asked for live stations for a scenario: compute nearest 4 from the
/// saved coord and reply over interconnect.
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

    match nearby(coord.lat, coord.lng, 4) {
        Ok(rows) => {
            let stations: Vec<serde_json::Value> = rows
                .iter()
                .map(|(name, bikes, docks, dist)| {
                    serde_json::json!({"name": name, "bikes": bikes, "docks": docks, "dist": dist})
                })
                .collect();
            let label = {
                let st = crate::state::state().lock().unwrap_or_else(|p| p.into_inner());
                st.config.coords[scenario]
                    .as_ref()
                    .map(|c| c.label.clone())
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| crate::state::SCENARIO_NAMES[scenario].to_string())
            };
            send_json(
                addr,
                serde_json::json!({
                    "tag": "stations",
                    "scenario": scenario,
                    "stations": stations,
                    "source": label,
                }),
            );
        }
        Err(err) => {
            send_json(addr, serde_json::json!({"tag":"stations","scenario":scenario,"error":err}));
        }
    }
}

/// Scenario 3 = "nearby": use the freshest phone GPS uploaded via /api/loc.
/// Falls back to a clear error the band can show when no recent fix exists.
pub fn reply_nearby_gps(addr: &str) {
    fn send_json(addr: &str, value: serde_json::Value) {
        use crate::astrobox::psys_host::interconnect;
        let text = value.to_string();
        let _ = wit_bindgen::block_on(
            interconnect::send_qaic_message(addr, crate::QA_PKG, &text).into_future(),
        );
    }

    let key = {
        let st = crate::state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.config.loc_key.clone()
    };
    match fetch_shortcut_location(&key) {
        Ok((lat, lng, age)) if age <= 900 => match nearby(lat, lng, 4) {
            Ok(rows) => {
                let stations: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|(name, bikes, docks, dist)| {
                        serde_json::json!({"name": name, "bikes": bikes, "docks": docks, "dist": dist})
                    })
                    .collect();
                let mins = age / 60;
                let label = if mins <= 0 {
                    "附近（剛剛定位）".to_string()
                } else {
                    format!("附近（{mins} 分鐘前定位）")
                };
                send_json(
                    addr,
                    serde_json::json!({
                        "tag": "stations",
                        "scenario": 3,
                        "stations": stations,
                        "source": label,
                    }),
                );
            }
            Err(err) => {
                send_json(addr, serde_json::json!({"tag":"stations","scenario":3,"error":err}));
            }
        },
        Ok((_, _, _)) => {
            send_json(addr, serde_json::json!({"tag":"stations","scenario":3,"error":"no-recent-gps"}));
        }
        Err(_) => {
            send_json(addr, serde_json::json!({"tag":"stations","scenario":3,"error":"no-recent-gps"}));
        }
    }
}

fn haversine_km(a_lat: f64, a_lng: f64, b_lat: f64, b_lng: f64) -> f64 {
    let r = 6371.0_f64;
    let dx = (b_lat - a_lat).to_radians();
    let dy = (b_lng - a_lng).to_radians();
    let h = (dx / 2.0).sin().powi(2)
        + a_lat.to_radians().cos() * b_lat.to_radians().cos() * (dy / 2.0).sin().powi(2);
    r * 2.0 * h.sqrt().atan2((1.0 - h).sqrt())
}
