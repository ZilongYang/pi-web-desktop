//! 更新进度窗口（L2 应用更新 / L3 pi-web 热更新共用）。
//!
//! 窗口本体是本地页面 `ui/update.html`（只在这个本地 origin 上才有 IPC），
//! Rust 侧负责开窗、播报进度、关窗。为什么需要它：更新动辄几十上百 MB，
//! 没有可见进度的话界面会静默十几秒，用户根本不知道发生了什么（真实反馈）。

use serde_json::Value;
use std::sync::Mutex;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const WINDOW_LABEL: &str = "update-progress";

static LAST: Mutex<Option<Value>> = Mutex::new(None);

pub fn open(app: &tauri::AppHandle) {
    if app.get_webview_window(WINDOW_LABEL).is_some() {
        return;
    }
    let built = WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        WebviewUrl::App("update.html".into()),
    )
    .title("Pi Web Desktop 更新")
    .inner_size(470.0, 210.0)
    .resizable(false)
    .center()
    .always_on_top(true)
    .build();
    if let Err(error) = built {
        crate::logging::line("progress", &format!("无法打开进度窗口：{error}"));
    }
}

pub fn close(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.close();
    }
}

/// phase: downloading / installing / ready / failed
pub fn emit(app: &tauri::AppHandle, phase: &str, mut payload: Value) {
    if let Some(map) = payload.as_object_mut() {
        map.insert("phase".into(), Value::String(phase.to_string()));
    }
    if let Ok(mut slot) = LAST.lock() {
        *slot = Some(payload.clone());
    }
    let _ = app.emit_to(WINDOW_LABEL, "update-progress", payload);
}

/// 进度页面加载后会主动拉一次（事件可能早于页面加载）。
#[tauri::command]
pub fn get_update_progress() -> Option<Value> {
    LAST.lock().ok().and_then(|value| value.clone())
}
