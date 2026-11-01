//! 原生菜单：默认菜单负责 macOS 的 ⌘C/⌘V/⌘A/⌘Z/⌘Q 这些"没有菜单就不生效"的标准
//! 快捷键（WKWebView 里尤其明显），自定义子菜单放壳自己的动作。

use tauri::menu::{Menu, MenuEvent, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Manager};

pub fn build(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;

    let reload = MenuItemBuilder::with_id("reload", "重新加载页面")
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    let restart = MenuItemBuilder::with_id("restart", "重启服务")
        .accelerator("CmdOrCtrl+Shift+R")
        .build(app)?;
    let open_browser = MenuItemBuilder::with_id("open_browser", "在浏览器中打开").build(app)?;
    let copy_url = MenuItemBuilder::with_id("copy_url", "复制服务地址").build(app)?;
    let open_logs = MenuItemBuilder::with_id("open_logs", "打开日志目录").build(app)?;
    let devtools = MenuItemBuilder::with_id("devtools", "开发者工具")
        .accelerator("CmdOrCtrl+Alt+I")
        .build(app)?;

    let submenu = SubmenuBuilder::new(app, "Pi Web")
        .items(&[&reload, &restart, &open_browser, &copy_url, &open_logs, &devtools])
        .build()?;
    menu.append(&submenu)?;
    Ok(menu)
}

pub fn on_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "reload" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.reload()");
            }
        }
        "restart" => {
            if let Err(error) = crate::restart_sidecar_now(app) {
                crate::logging::line("menu", &format!("重启失败：{error}"));
            }
        }
        "open_browser" => {
            if let Some(url) = crate::current_url(app) {
                use tauri_plugin_opener::OpenerExt;
                let _ = app.opener().open_url(url, None::<&str>);
            }
        }
        "copy_url" => {
            // 复制到剪贴板需要剪贴板插件；M1 先把地址打进日志，避免多引一个依赖
            if let Some(url) = crate::current_url(app) {
                crate::logging::line("menu", &format!("服务地址：{url}"));
            }
        }
        "open_logs" => {
            if let Some(path) = crate::logging::path() {
                use tauri_plugin_opener::OpenerExt;
                if let Some(dir) = path.parent() {
                    let _ = app.opener().open_path(dir.to_string_lossy().to_string(), None::<&str>);
                }
            }
        }
        "devtools" => {
            if let Some(window) = app.get_webview_window("main") {
                if window.is_devtools_open() {
                    window.close_devtools();
                } else {
                    window.open_devtools();
                }
            }
        }
        _ => {}
    }
}
