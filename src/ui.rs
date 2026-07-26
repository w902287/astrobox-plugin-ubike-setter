//! Plugin UI: scenario tabs -> station search -> pick from list -> push to band.

use crate::astrobox::psys_host::ui_v3 as ui;
use crate::state::{self, Coord, SCENARIO_NAMES};
use crate::stations;

const EV_TAB0: &str = "tab0";
const EV_TAB1: &str = "tab1";
const EV_TAB2: &str = "tab2";
const EV_QUERY: &str = "query_input";
const EV_SEARCH: &str = "do_search";
const EV_PUSH: &str = "push_now";
const EV_CLEAR: &str = "clear_current";
const EV_LOCATE: &str = "locate_me";
const EV_RUN_SHORTCUT: &str = "run_shortcut";
const EV_TRIGGER_INPUT: &str = "trigger_input";
const EV_PICK_PREFIX: &str = "pick_";

const C_BG: &str = "#101914";
const C_CARD: &str = "#171f1a";
const C_GREEN: &str = "#63d987";
const C_TEXT: &str = "#eef7f0";
const C_MUTED: &str = "#9aaba0";
const C_DARK_TEXT: &str = "#091006";

pub fn render_root(element_id: &str) {
    {
        let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.root_element = Some(element_id.to_string());
    }
    ui::render(element_id, build_ui());
}

pub fn rerender() {
    let root = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.root_element.clone()
    };
    if let Some(root) = root {
        ui::render(&root, build_ui());
    }
}

pub fn handle_ui_event(event_id: &str, ev: ui::Event, payload: &str) {
    match ev {
        ui::Event::Click => match event_id {
            EV_TAB0 => set_tab(0),
            EV_TAB1 => set_tab(1),
            EV_TAB2 => set_tab(2),
            EV_SEARCH => do_search(),
            EV_LOCATE => do_locate(),
            EV_RUN_SHORTCUT => run_shortcut(),
            EV_PUSH => {
                state::push_config_to("");
                state::set_notice("已推送設定到手環");
                rerender();
            }
            EV_CLEAR => {
                {
                    let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
                    let idx = st.selected_scenario;
                    st.config.coords[idx] = None;
                }
                state::save();
                state::set_notice("已清除此地點");
                rerender();
            }
            other => {
                if let Some(rest) = other.strip_prefix(EV_PICK_PREFIX) {
                    if let Ok(i) = rest.parse::<usize>() {
                        pick_station(i);
                    }
                }
            }
        },
        ui::Event::Input | ui::Event::Change => {
            if event_id == EV_QUERY {
                let text = extract_input_value(payload);
                let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
                st.query = text;
            } else if event_id == EV_TRIGGER_INPUT {
                let text = extract_input_value(payload);
                {
                    let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
                    st.config.android_trigger_url = text.trim().to_string();
                }
                state::save();
            }
        }
        _ => {}
    }
}

fn extract_input_value(payload: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
        for key in ["value", "content", "text"] {
            if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                return s.to_string();
            }
        }
        if let Some(s) = v.as_str() {
            return s.to_string();
        }
    }
    payload.to_string()
}

fn set_tab(i: usize) {
    {
        let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.selected_scenario = i;
    }
    rerender();
}

fn do_search() {
    let query = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.query.clone()
    };
    if query.trim().is_empty() {
        state::set_notice("請輸入地點，例如：市政府、台北車站、公司地址");
        rerender();
        return;
    }
    state::set_notice("搜尋地點中…");
    rerender();
    match stations::geocode_smart(&query, 6) {
        Ok((results, used)) => {
            let count = results.len();
            let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
            st.results = results;
            st.notice = if count == 0 {
                Some("找不到這個地點；完整地址可試著只留到「號」或用路口／地標".to_string())
            } else if let Some(used) = used {
                Some(format!("原地址查無，已用「{used}」找到 {count} 個結果，點選即設定"))
            } else {
                Some(format!("找到 {count} 個地點，點選即設定並推送"))
            };
        }
        Err(err) => {
            state::set_notice(format!("搜尋失敗：{err}"));
        }
    }
    rerender();
}

fn run_shortcut() {
    use crate::astrobox::psys_host::dialog;
    let (platform, trigger) = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        (st.platform.clone(), st.config.android_trigger_url.clone())
    };
    let is_android = platform.contains("android");
    if is_android {
        if trigger.is_empty() {
            state::set_notice(
                "Android：請先在下方「自動化觸發連結」填入 HTTP Shortcuts 的觸發網址（見底部說明）",
            );
            rerender();
            return;
        }
        {
            let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
            st.awaiting_gps = true;
        }
        dialog::open_url(trigger.as_str());
        state::set_notice("已喚起定位任務；完成後回到本頁會自動套用");
    } else {
        {
            let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
            st.awaiting_gps = true;
        }
        dialog::open_url("shortcuts://run-shortcut?name=UbikeGPS");
        state::set_notice("已喚起「UbikeGPS」捷徑取得定位；回到本頁會自動套用");
    }
    rerender();
}

/// Called on every UI render. If we just launched the GPS shortcut, try to
/// pull the freshly uploaded location and apply it automatically.
pub fn auto_pull_gps_if_awaiting() {
    let (awaiting, key) = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        (st.awaiting_gps, st.config.loc_key.clone())
    };
    if !awaiting {
        return;
    }
    if let Ok((lat, lng, age)) = stations::fetch_shortcut_location(&key) {
        if age <= 120 {
            {
                let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
                st.awaiting_gps = false;
            }
            apply_location(lat, lng, format!("手機 GPS（{age} 秒前）"));
        }
    }
}

fn do_locate() {
    let key = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.config.loc_key.clone()
    };
    state::set_notice("讀取手機定位中…");
    rerender();
    match stations::fetch_shortcut_location(&key) {
        Ok((lat, lng, age)) => {
            if age <= 600 {
                apply_location(lat, lng, format!("手機 GPS（{age} 秒前）"));
            } else {
                state::set_notice(format!(
                    "定位太舊（{} 分鐘前），請重新執行手機捷徑後再按一次",
                    age / 60
                ));
                rerender();
            }
        }
        Err(err) => {
            state::set_notice(format!("{err}。請先執行手機定位捷徑，再按「目前位置」"));
            rerender();
        }
    }
}

fn apply_location(lat: f64, lng: f64, desc: String) {
    let scenario = {
        let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        let idx = st.selected_scenario;
        st.config.coords[idx] = Some(Coord {
            lat,
            lng,
            label: format!("目前位置 {:.4},{:.4}", lat, lng),
        });
        st.results.clear();
        idx
    };
    state::save();
    state::push_config_to("");
    let preview = stations::nearby(lat, lng, 4).unwrap_or_default();
    let preview_text = preview
        .iter()
        .map(|(n, _b, _d, dist)| format!("{}（{}m）", n, dist))
        .collect::<Vec<_>>()
        .join("；");
    state::set_notice(format!(
        "{}已設為{}並推送。附近：{}",
        SCENARIO_NAMES[scenario], desc,
        if preview_text.is_empty() { "讀取中".to_string() } else { preview_text }
    ));
    rerender();
}

fn pick_station(i: usize) {
    let picked = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.results.get(i).cloned()
    };
    let Some((name, lat, lng)) = picked else {
        return;
    };
    let scenario = {
        let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        let idx = st.selected_scenario;
        st.config.coords[idx] = Some(Coord {
            lat,
            lng,
            label: name.clone(),
        });
        st.results.clear();
        idx
    };
    state::save();
    state::push_config_to("");
    // preview nearest stations around the picked place
    let preview = stations::nearby(lat, lng, 4).unwrap_or_default();
    let preview_text = preview
        .iter()
        .map(|(n, b, d, dist)| format!("{}（可借{} 可還{}・{}m）", n, b, d, dist))
        .collect::<Vec<_>>()
        .join("；");
    {
        let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.preview = preview_text.clone();
    }
    state::set_notice(format!(
        "{}已設為「{}」，已推送。附近站點：{}",
        SCENARIO_NAMES[scenario], name,
        if preview_text.is_empty() { "（讀取中）".to_string() } else { preview_text }
    ));
    rerender();
}

fn build_ui() -> ui::Element {
    let st = state::state().lock().unwrap_or_else(|p| p.into_inner());

    let title = ui::Element::new(ui::ElementType::P, Some("Ubike助手 設定"))
        .size(22)
        .text_color(C_GREEN);

    let device_line = if st.devices.is_empty() {
        "未偵測到已連線手環".to_string()
    } else {
        format!(
            "已連線：{}",
            st.devices
                .iter()
                .map(|(_, n)| n.as_str())
                .collect::<Vec<_>>()
                .join("、")
        )
    };
    let device_el = ui::Element::new(ui::ElementType::P, Some(device_line.as_str()))
        .size(13)
        .text_color(C_MUTED);

    // Scenario tabs
    let mut tabs = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Row)
        .gap(8)
        .margin_top(10);
    for (i, name) in SCENARIO_NAMES.iter().enumerate() {
        let ev = [EV_TAB0, EV_TAB1, EV_TAB2][i];
        let active = st.selected_scenario == i;
        let mut b = ui::Element::new(ui::ElementType::Button, Some(name))
            .radius(16)
            .padding_left(14)
            .padding_right(14)
            .on(ui::Event::Click, ev);
        b = if active {
            b.bg(C_GREEN).text_color(C_DARK_TEXT)
        } else {
            b.bg(C_CARD).text_color(C_TEXT)
        };
        tabs = tabs.child(b);
    }

    // Current saved config lines
    let mut saved_col = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .gap(4)
        .margin_top(8);
    for (i, name) in SCENARIO_NAMES.iter().enumerate() {
        let text = match &st.config.coords[i] {
            Some(c) if !c.label.is_empty() => format!("{}：{}", name, c.label),
            Some(c) => format!("{}：{:.4}, {:.4}", name, c.lat, c.lng),
            None => format!("{}：未設定", name),
        };
        let color = if st.config.coords[i].is_some() {
            C_GREEN
        } else {
            C_MUTED
        };
        saved_col = saved_col.child(
            ui::Element::new(ui::ElementType::P, Some(text.as_str()))
                .size(14)
                .text_color(color),
        );
    }

    // Search row
    let input = ui::Element::new(ui::ElementType::Input, Some(st.query.as_str()))
        .prop("placeholder", "地點或地址：市政府／信義區市府路45號")
        .width_full()
        .on(ui::Event::Input, EV_QUERY)
        .on(ui::Event::Change, EV_QUERY);
    let search_btn = ui::Element::new(ui::ElementType::Button, Some("搜尋地點"))
        .bg(C_GREEN)
        .text_color(C_DARK_TEXT)
        .radius(10)
        .on(ui::Event::Click, EV_SEARCH);
    let shortcut_btn = ui::Element::new(ui::ElementType::Button, Some("手機定位"))
        .bg(C_CARD)
        .text_color(C_GREEN)
        .radius(10)
        .on(ui::Event::Click, EV_RUN_SHORTCUT);
    let locate_btn = ui::Element::new(ui::ElementType::Button, Some("目前位置"))
        .bg(C_CARD)
        .text_color(C_TEXT)
        .radius(10)
        .on(ui::Event::Click, EV_LOCATE);
    let search_row = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Row)
        .gap(8)
        .margin_top(10)
        .child(input)
        .child(search_btn)
        .child(shortcut_btn)
        .child(locate_btn);

    // Notice
    let notice_el = st.notice.as_ref().map(|n| {
        ui::Element::new(ui::ElementType::P, Some(n.as_str()))
            .size(13)
            .text_color(C_GREEN)
            .margin_top(6)
    });

    // Results list
    let mut results_col = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .gap(6)
        .margin_top(8);
    for (i, (name, _lat, _lng)) in st.results.iter().enumerate() {
        let label = name.clone();
        let ev = format!("{}{}", EV_PICK_PREFIX, i);
        results_col = results_col.child(
            ui::Element::new(ui::ElementType::Button, Some(label.as_str()))
                .bg(C_CARD)
                .text_color(C_TEXT)
                .radius(10)
                .width_full()
                .padding(10)
                .on(ui::Event::Click, ev.as_str()),
        );
    }

    // Bottom actions
    let push_btn = ui::Element::new(ui::ElementType::Button, Some("重新推送設定到手環"))
        .bg(C_CARD)
        .text_color(C_TEXT)
        .radius(10)
        .on(ui::Event::Click, EV_PUSH);
    let clear_btn = ui::Element::new(ui::ElementType::Button, Some("清除目前分頁地點"))
        .bg(C_CARD)
        .text_color(C_MUTED)
        .radius(10)
        .on(ui::Event::Click, EV_CLEAR);
    let actions = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Row)
        .gap(8)
        .margin_top(12)
        .child(push_btn)
        .child(clear_btn);

    let is_android = st.platform.contains("android");
    let shortcut_hint = {
        let key = st.config.loc_key.clone();
        let text = if is_android {
            format!(
                "Android 手機GPS：安裝 HTTP Shortcuts，建一個「取得定位後 POST」的捷徑（POST https://youbike-band.w902287.workers.dev/api/loc，JSON：key={key}、lat={{location.latitude}}、lng={{location.longitude}}），長按捷徑→「觸發連結」複製後貼到下方欄位。"
            )
        } else {
            format!(
                "iOS 手機GPS：建立名為 UbikeGPS 的捷徑（取得目前位置 → POST 到 /api/loc，key={key}）。按「手機定位」自動執行並回來套用。"
            )
        };
        ui::Element::new(ui::ElementType::P, Some(text.as_str()))
            .size(11)
            .text_color(C_MUTED)
            .margin_top(10)
    };
    let android_trigger_row = if is_android {
        let input = ui::Element::new(
            ui::ElementType::Input,
            Some(st.config.android_trigger_url.as_str()),
        )
        .prop("placeholder", "貼上 HTTP Shortcuts 觸發連結（http-shortcuts://...）")
        .width_full()
        .on(ui::Event::Input, EV_TRIGGER_INPUT)
        .on(ui::Event::Change, EV_TRIGGER_INPUT);
        Some(
            ui::Element::new(ui::ElementType::Div, None)
                .flex()
                .flex_direction(ui::FlexDirection::Column)
                .margin_top(6)
                .child(
                    ui::Element::new(ui::ElementType::P, Some("自動化觸發連結（Android）"))
                        .size(12)
                        .text_color(C_MUTED),
                )
                .child(input),
        )
    } else {
        None
    };

    let mut root = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .width_full()
        .bg(C_BG)
        .padding(14)
        .radius(12)
        .child(title)
        .child(device_el)
        .child(tabs)
        .child(saved_col)
        .child(search_row);
    if let Some(n) = notice_el {
        root = root.child(n);
    }
    root = root.child(results_col).child(actions).child(shortcut_hint);
    if let Some(row) = android_trigger_row {
        root = root.child(row);
    }
    root
}
