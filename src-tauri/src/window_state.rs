//! 主窗口位置/大小的记忆（F19）。
//!
//! 把「普通态」（非最大化、非全屏）的窗口位置与大小写进 `config.json`，下次启动
//! 恢复到同一位置；恢复前做一次可见性校验，避免换显示器/拔掉外接屏后把窗口丢在
//! 屏幕外面（那时改为居中）。
//!
//! 坐标一律用**物理像素**（`outer_position` + `inner_size`）：逻辑坐标在跨显示器
//! （scale factor 不同）时不唯一，物理坐标才稳定。所有错误只记日志，绝不让窗口
//! 事件里 panic —— 这个模块的全部代码都在用户拖窗口的热路径上。

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, PhysicalPosition, PhysicalSize, WebviewWindow};

/// 拖动/缩放会连续触发事件，写盘节流到 800ms 一次；关窗/退出时强制写。
const SAVE_INTERVAL: Duration = Duration::from_millis(800);
/// 恢复时的尺寸下限（异常的旧值直接忽略，用 tauri.conf.json 的默认值）。
const MIN_WIDTH: u32 = 200;
const MIN_HEIGHT: u32 = 150;

static LAST_SAVE: Mutex<Option<Instant>> = Mutex::new(None);

/// 恢复上次的位置与大小（在主窗口 build 之后、show 之前调用，避免可见跳动）。
pub fn restore(app: &AppHandle, window: &WebviewWindow) {
    let config = crate::sidecar::Config::load(app);
    let Some(geometry) = config.window else { return };
    if geometry.width < MIN_WIDTH || geometry.height < MIN_HEIGHT {
        crate::logging::line(
            "window",
            &format!("上次记录的窗口尺寸异常（{}×{}），忽略", geometry.width, geometry.height),
        );
        return;
    }

    if let Err(error) = window.set_position(PhysicalPosition::new(geometry.x, geometry.y)) {
        crate::logging::line("window", &format!("恢复窗口位置失败：{error}"));
    }
    if let Err(error) = window.set_size(PhysicalSize::new(geometry.width, geometry.height)) {
        crate::logging::line("window", &format!("恢复窗口大小失败：{error}"));
        return;
    }

    if on_screen(window) {
        crate::logging::line(
            "window",
            &format!(
                "恢复窗口 {}×{} @ ({}, {})",
                geometry.width, geometry.height, geometry.x, geometry.y
            ),
        );
    } else {
        let _ = window.center();
        crate::logging::line("window", "上次的位置已不在任何屏幕内，改为居中显示");
    }
}

/// 给主窗口挂上「移动/缩放/关闭时记住几何」的钩子。
pub fn watch(app: &AppHandle, window: &WebviewWindow) {
    let app = app.clone();
    let watched = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            save(&app, &watched, false)
        }
        tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
            save(&app, &watched, true)
        }
        _ => {}
    });
}

/// 保存当前几何。`force` = 关窗/退出时立即写（不做节流）。
pub fn save(app: &AppHandle, window: &WebviewWindow, force: bool) {
    // 最大化/全屏时拿到的是整屏尺寸，记下来没意义，也会把用户原来的普通尺寸覆盖掉。
    if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        return;
    }

    if !force {
        let Ok(mut last) = LAST_SAVE.lock() else { return };
        let now = Instant::now();
        if let Some(previous) = *last {
            if now.duration_since(previous) < SAVE_INTERVAL {
                return;
            }
        }
        *last = Some(now);
    }

    let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
        return;
    };
    let geometry = crate::sidecar::WindowGeometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    };

    let mut config = crate::sidecar::Config::load(app);
    if config.window == Some(geometry) {
        return;
    }
    config.window = Some(geometry);
    config.save(app);
    if force {
        crate::logging::line(
            "window",
            &format!(
                "已记住窗口 {}×{} @ ({}, {})",
                geometry.width, geometry.height, geometry.x, geometry.y
            ),
        );
    }
}

/// 测试探针（`PIWEB_WINDOW_PROBE=1` 时由 main.rs 调用）：打印当前几何、强制落盘，
/// 供 scripts/smoke-app.mjs 在没有 GUI 自动化权限的环境里验证「启动不崩 + 几何存取」。
pub fn probe_and_save(app: &AppHandle, window: &WebviewWindow) {
    match (window.outer_position(), window.inner_size()) {
        (Ok(position), Ok(size)) => crate::logging::line(
            "window-probe",
            &format!(
                "{}x{} @ ({}, {})",
                size.width, size.height, position.x, position.y
            ),
        ),
        _ => crate::logging::line("window-probe", "拿不到窗口几何"),
    }
    save(app, window, true);
}

/// 窗口是否还有足够大的一块区域落在某块屏幕里（默认 true：拿不到显示器信息时不做判定）。
fn on_screen(window: &WebviewWindow) -> bool {
    let (Ok(position), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return true;
    };
    let Ok(monitors) = window.available_monitors() else { return true };
    if monitors.is_empty() {
        return true;
    }
    monitors.iter().any(|monitor| {
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        let left = position.x.max(monitor_position.x);
        let top = position.y.max(monitor_position.y);
        let right = (position.x + size.width as i32)
            .min(monitor_position.x + monitor_size.width as i32);
        let bottom = (position.y + size.height as i32)
            .min(monitor_position.y + monitor_size.height as i32);
        // 至少露出 120×80，用户才抓得住标题栏
        right - left >= 120 && bottom - top >= 80
    })
}
