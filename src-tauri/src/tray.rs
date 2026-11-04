//! 菜单栏托盘（F16）+ 关窗隐藏到菜单栏。
//!
//! 用户 2026-11-03 定的行为 **B**：点关闭按钮不退出，而是隐藏窗口；只有
//! 托盘/菜单里的「退出 Pi Web Desktop」才真正停侧车退出（pi-web 的后台任务不被中断）。
//!
//! 图标用模板图（`icons/tray-template.png`，π 字形）+ `icon_as_template(true)`，
//! 由系统按深浅色菜单栏自动反色。

use tauri::image::Image;
use tauri::menu::{Menu, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

/// 菜单栏图标（编译期嵌入，36×36 = 18pt @2x，模板图）
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray-template.png");

/// 建托盘图标与菜单。setup 里调用一次。
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItemBuilder::with_id("tray_toggle", "显示 / 隐藏主窗口").build(app)?;
    let open_browser = MenuItemBuilder::with_id("open_browser", "在浏览器中打开").build(app)?;
    let check_update = MenuItemBuilder::with_id("check_update", "检查应用更新…").build(app)?;
    let open_logs = MenuItemBuilder::with_id("open_logs", "打开日志目录").build(app)?;
    let quit = MenuItemBuilder::with_id("tray_quit", "退出 Pi Web Desktop").build(app)?;

    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &open_browser,
            &check_update,
            &open_logs,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let icon = Image::from_bytes(TRAY_ICON_PNG)?;
    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Pi Web Desktop")
        .menu(&menu)
        .show_menu_on_left_click(true)
        // 注意：这里**不要**再注册 on_menu_event。
        // 托盘菜单的点击同样会走到 App 的全局菜单监听（Builder::on_menu_event），
        // 两处都处理会对同一次点击执行两遍（0.3.1-beta.1 的 bug：检查更新弹两个窗）。
        // 托盘专有的 id（tray_toggle / tray_quit）统一在 menu::on_event 里处理。
        .build(app)?;
    crate::logging::line("tray", "菜单栏托盘已创建");
    Ok(())
}

/// 显示 / 隐藏主窗口。
pub fn toggle_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    if visible {
        let _ = window.hide();
        crate::logging::line("tray", "窗口已隐藏（菜单栏可恢复）");
    } else {
        let _ = window.show();
        let _ = window.set_focus();
        crate::logging::line("tray", "窗口已显示");
    }
}
