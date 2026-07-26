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
        state::set_notice("請先輸入站名或路名關鍵字");
        rerender();
        return;
    }
    state::set_notice("搜尋中…");
    rerender();
    match stations::search(&query, 12) {
        Ok(results) => {
            let count = results.len();
            let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
            st.results = results;
            st.notice = if count == 0 {
                Some("找不到符合的站點".to_string())
            } else {
                Some(format!("找到 {count} 個站點，點選以設定"))
            };
        }
        Err(err) => {
            state::set_notice(format!("搜尋失敗：{err}"));
        }
    }
    rerender();
}

fn pick_station(i: usize) {
    let picked = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.results.get(i).cloned()
    };
    let Some((name, lat, lng, _area)) = picked else {
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
        idx
    };
    state::save();
    state::push_config_to("");
    state::set_notice(format!(
        "{}已設為「{}」並推送到手環",
        SCENARIO_NAMES[scenario], name
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
        .prop("placeholder", "輸入站名／路名，例如 市政府")
        .width_full()
        .on(ui::Event::Input, EV_QUERY)
        .on(ui::Event::Change, EV_QUERY);
    let search_btn = ui::Element::new(ui::ElementType::Button, Some("搜尋站點"))
        .bg(C_GREEN)
        .text_color(C_DARK_TEXT)
        .radius(10)
        .on(ui::Event::Click, EV_SEARCH);
    let search_row = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Row)
        .gap(8)
        .margin_top(10)
        .child(input)
        .child(search_btn);

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
    for (i, (name, _lat, _lng, area)) in st.results.iter().enumerate() {
        let label = if area.is_empty() {
            name.clone()
        } else {
            format!("{}（{}）", name, area)
        };
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
    root = root.child(results_col).child(actions);
    root
}
