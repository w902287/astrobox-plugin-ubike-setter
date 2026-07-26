//! Minimal FetchBridge-compatible fetch proxy (v1 single-frame protocol).
//!
//! Band sends `{tag:"fetch", id, url, options:{method,headers,body}}`;
//! we reply `{tag:"fetch", id, resp:{ok,status,body,headers}}` — exactly the
//! shape the quick app's bridgeFetch expects. This makes the plugin a full
//! replacement for the FetchBridge plugin for `tw.youbike.band`, so the old
//! Worker-based RPK keeps working and no second plugin is needed.

use std::str::FromStr;

use serde_json::{json, Map, Value};
use waki::{Client, Method};

const MAX_BODY_LEN: usize = 64 * 1024;

pub fn handle_fetch(addr: &str, req: Value) {
    let id = req
        .get("id")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let url = req
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if url.is_empty() {
        return;
    }
    let opts = req.get("options").cloned().unwrap_or_else(|| json!({}));
    let method = opts
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let body = opts
        .get("body")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let headers: Vec<(String, String)> = opts
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| {
                    let val = match v {
                        Value::String(s) => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        Value::Bool(b) => Some(b.to_string()),
                        _ => None,
                    };
                    val.map(|s| (k.clone(), s))
                })
                .collect()
        })
        .unwrap_or_default();

    let m = match method.as_str() {
        "POST" => Method::Post,
        "PUT" => Method::Put,
        "DELETE" => Method::Delete,
        "PATCH" => Method::Patch,
        "HEAD" => Method::Head,
        "OPTIONS" => Method::Options,
        _ => Method::Get,
    };

    let mut builder = Client::new()
        .request(m, &url)
        .connect_timeout(std::time::Duration::from_secs(15));
    if !headers.is_empty() {
        let pairs: Vec<(http::header::HeaderName, String)> = headers
            .iter()
            .filter_map(|(k, v)| {
                http::header::HeaderName::from_str(k.as_str())
                    .ok()
                    .map(|name| (name, v.clone()))
            })
            .collect();
        builder = builder.headers(pairs);
    }
    if let Some(b) = body {
        builder = builder.body(b.into_bytes());
    }

    let resp_json = match builder.send() {
        Ok(resp) => {
            let status = resp.status_code();
            let mut hdrs = Map::<String, Value>::new();
            {
                let headers = resp.headers();
                for (k, v) in headers.iter() {
                    if let Ok(text) = v.to_str() {
                        hdrs.insert(k.as_str().to_string(), Value::String(text.to_string()));
                    }
                }
            }
            let bytes = resp.body().unwrap_or_default();
            let mut text = String::from_utf8_lossy(&bytes).to_string();
            if text.len() > MAX_BODY_LEN {
                text.truncate(MAX_BODY_LEN);
            }
            json!({
                "tag": "fetch",
                "id": id,
                "resp": {
                    "ok": (200..300).contains(&status),
                    "status": status,
                    "body": text,
                    "headers": hdrs,
                }
            })
        }
        Err(err) => json!({
            "tag": "fetch",
            "id": id,
            "resp": { "ok": false, "status": 0, "body": format!("{err}"), "headers": {} }
        }),
    };

    let text = resp_json.to_string();
    let _ = wit_bindgen::block_on(
        crate::astrobox::psys_host::interconnect::send_qaic_message(addr, crate::QA_PKG, &text)
            .into_future(),
    );
}
