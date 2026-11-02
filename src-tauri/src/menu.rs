//! 原生菜单（全中文）。
//!
//! Tauri 的 `Menu::default()` 只给英文的 File/Edit/Window/Help，而且**没有 View
//! 菜单、没有页面缩放**；而 macOS 上 ⌘C/⌘V/⌘Z/⌘Q 这些标准快捷键必须挂在菜单项上
//! 才生效。所以这里自己拼一套：
//!
//!   Pi Web Desktop（关于/服务/隐藏/退出）· 文件 · 编辑 · 视图（放大/缩小/实际大小/全屏）
//!   · 窗口 · Pi Web（壳自己的动作）· 帮助（GitHub 仓库）
//!
//! 编辑菜单用 `PredefinedMenuItem`，走 AppKit 的标准 selector，快捷键和系统行为都一致。

use tauri::menu::{AboutMetadata, Menu, MenuItemBuilder, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager};

pub fn build(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    let package = app.package_info();

    // App 菜单（macOS 上必须是第一项）
    #[cfg(target_os = "macos")]
    {
        let about_metadata = AboutMetadata {
            name: Some("Pi Web Desktop".to_string()),
            version: Some(package.version.to_string()),
            copyright: None,
            authors: None,
            ..Default::default()
        };
        let app_submenu = Submenu::with_items(
            app,
            "Pi Web Desktop",
            true,
            &[
                &PredefinedMenuItem::about(app, Some("关于 Pi Web Desktop"), Some(about_metadata))?,
                // macOS 惯例：关于下面紧跟「检查更新…」
                &MenuItemBuilder::with_id("check_update", "检查更新…").build(app)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, Some("服务"))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, Some("隐藏 Pi Web Desktop"))?,
                &PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?,
                &PredefinedMenuItem::show_all(app, Some("显示全部"))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, Some("退出 Pi Web Desktop"))?,
            ],
        )?;
        menu.append(&app_submenu)?;
    }

    // 文件
    let file_menu = Submenu::with_items(
        app,
        "文件",
        true,
        &[
            &PredefinedMenuItem::close_window(app, Some("关闭窗口"))?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::quit(app, Some("退出 Pi Web Desktop"))?,
        ],
    )?;
    menu.append(&file_menu)?;

    // 编辑（⌘Z / ⌘X / ⌘C / ⌘V / ⌘A 全靠这些预定义项）
    let edit_menu = Submenu::with_items(
        app,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("撤销"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("剪切"))?,
            &PredefinedMenuItem::copy(app, Some("拷贝"))?,
            &PredefinedMenuItem::paste(app, Some("粘贴"))?,
            &PredefinedMenuItem::select_all(app, Some("全选"))?,
        ],
    )?;
    menu.append(&edit_menu)?;

    // 视图：页面缩放（⌘= / ⌘- / ⌘0）+ 全屏
    let zoom_in = MenuItemBuilder::with_id("zoom_in", "放大")
        .accelerator("CmdOrCtrl+=")
        .build(app)?;
    let zoom_out = MenuItemBuilder::with_id("zoom_out", "缩小")
        .accelerator("CmdOrCtrl+-")
        .build(app)?;
    let zoom_reset = MenuItemBuilder::with_id("zoom_reset", "实际大小")
        .accelerator("CmdOrCtrl+0")
        .build(app)?;
    let view_menu = Submenu::with_items(
        app,
        "视图",
        true,
        &[
            &zoom_in,
            &zoom_out,
            &zoom_reset,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, Some("进入全屏"))?,
        ],
    )?;
    menu.append(&view_menu)?;

    // 窗口
    let window_menu = Submenu::with_items(
        app,
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("最小化"))?,
            &PredefinedMenuItem::maximize(app, Some("缩放窗口"))?,
        ],
    )?;
    menu.append(&window_menu)?;

    // Pi Web 自己的动作
    let reload = MenuItemBuilder::with_id("reload", "重新加载页面")
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    let restart = MenuItemBuilder::with_id("restart", "重启服务")
        .accelerator("CmdOrCtrl+Shift+R")
        .build(app)?;
    let check_update = MenuItemBuilder::with_id("check_update", "检查更新…").build(app)?;
    let open_browser = MenuItemBuilder::with_id("open_browser", "在浏览器中打开").build(app)?;
    let copy_url = MenuItemBuilder::with_id("copy_url", "复制服务地址").build(app)?;
    let open_logs = MenuItemBuilder::with_id("open_logs", "打开日志目录").build(app)?;
    let devtools = MenuItemBuilder::with_id("devtools", "开发者工具")
        .accelerator("CmdOrCtrl+Alt+I")
        .build(app)?;
    let piweb_menu = Submenu::with_items(
        app,
        "Pi Web",
        true,
        &[
            &reload,
            &restart,
            &check_update,
            &PredefinedMenuItem::separator(app)?,
            &open_browser,
            &copy_url,
            &open_logs,
            &PredefinedMenuItem::separator(app)?,
            &devtools,
        ],
    )?;
    menu.append(&piweb_menu)?;

    // 帮助
    let open_repo = MenuItemBuilder::with_id("open_repo", "GitHub 仓库").build(app)?;
    let help_menu = Submenu::with_items(app, "帮助", true, &[&open_repo])?;
    menu.append(&help_menu)?;

    Ok(menu)
}

pub fn on_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
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
        "check_update" => crate::update::manual_check_all(app),
        "open_browser" => {
            if let Some(url) = crate::current_url(app) {
                use tauri_plugin_opener::OpenerExt;
                let _ = app.opener().open_url(url, None::<&str>);
            }
        }
        "copy_url" => {
            if let Some(url) = crate::current_url(app) {
                match crate::update::copy_to_clipboard_public(&url) {
                    Ok(()) => crate::logging::line("menu", "服务地址已复制到剪贴板"),
                    Err(error) => crate::logging::line("menu", &format!("复制失败：{error}")),
                }
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
        "zoom_in" => crate::zoom::zoom_in(app),
        "zoom_out" => crate::zoom::zoom_out(app),
        "zoom_reset" => crate::zoom::zoom_reset(app),
        "open_repo" => {
            use tauri_plugin_opener::OpenerExt;
            let _ = app.opener().open_url(
                "https://github.com/ZilongYang/pi-web-desktop",
                None::<&str>,
            );
        }
        _ => {}
    }
}
