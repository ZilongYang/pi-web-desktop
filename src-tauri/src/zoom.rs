//! 页面缩放：菜单「视图 → 放大/缩小/实际大小」，⌘= / ⌘- / ⌘0。
//!
//! pi-web 是网页 UI，字号偏小的时候需要缩放。这里用 `WebviewWindow::set_zoom`
//! （macOS 上就是 WKWebView 的 pageZoom），按固定档位走，避免每次 ±10% 出现
//! 100.0001% 这种怪数字；当前档位会写回 config.json，重启后保持。

use std::sync::Mutex;
use tauri::Manager;

/// 固定档位：50% → 300%。索引 4 = 100%。
const STEPS: [f64; 12] = [
    0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];
const DEFAULT_INDEX: usize = 4;

static INDEX: Mutex<usize> = Mutex::new(DEFAULT_INDEX);

fn nearest_index(zoom: f64) -> usize {
    let mut best = DEFAULT_INDEX;
    let mut best_delta = f64::MAX;
    for (index, step) in STEPS.iter().enumerate() {
        let delta = (step - zoom).abs();
        if delta < best_delta {
            best_delta = delta;
            best = index;
        }
    }
    best
}

fn apply(app: &tauri::AppHandle) -> Result<(), String> {
    let index = *INDEX.lock().map_err(|_| "缩放状态锁失败".to_string())?;
    let factor = STEPS[index];
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;
    window.set_zoom(factor).map_err(|error| error.to_string())?;
    crate::logging::line("zoom", &format!("页面缩放 {:.0}%", factor * 100.0));

    // 记住这个档位（下次启动恢复）
    let mut config = crate::sidecar::Config::load(app);
    if (config.zoom - factor).abs() > f64::EPSILON {
        config.zoom = factor;
        config.save(app);
    }
    Ok(())
}

pub fn zoom_in(app: &tauri::AppHandle) {
    if let Ok(mut index) = INDEX.lock() {
        *index = (*index + 1).min(STEPS.len() - 1);
    }
    if let Err(error) = apply(app) {
        crate::logging::line("zoom", &format!("放大失败：{error}"));
    }
}

pub fn zoom_out(app: &tauri::AppHandle) {
    if let Ok(mut index) = INDEX.lock() {
        *index = index.saturating_sub(1);
    }
    if let Err(error) = apply(app) {
        crate::logging::line("zoom", &format!("缩小失败：{error}"));
    }
}

pub fn zoom_reset(app: &tauri::AppHandle) {
    if let Ok(mut index) = INDEX.lock() {
        *index = DEFAULT_INDEX;
    }
    if let Err(error) = apply(app) {
        crate::logging::line("zoom", &format!("恢复实际大小失败：{error}"));
    }
}

/// 启动时恢复上次的缩放档位。
pub fn restore(app: &tauri::AppHandle) {
    let config = crate::sidecar::Config::load(app);
    if let Ok(mut index) = INDEX.lock() {
        *index = nearest_index(config.zoom);
    }
    if let Err(error) = apply(app) {
        crate::logging::line("zoom", &format!("恢复缩放失败：{error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_nearest_step() {
        assert_eq!(nearest_index(1.0), 4);
        assert_eq!(nearest_index(1.04), 4);
        assert_eq!(nearest_index(1.2), 6); // 1.25
        assert_eq!(nearest_index(9.0), STEPS.len() - 1);
        assert_eq!(nearest_index(0.1), 0);
    }
}
