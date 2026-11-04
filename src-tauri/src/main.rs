#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Pi Web Desktop —— pi-web 的 Tauri 2 壳。
//!
//! 一次启动的完整链路：
//!   splash 窗口 → 清理上次孤儿 → 附着同版本实例 或 起侧车 → 探活 `GET /`
//!   → 导航到 http://127.0.0.1:<port>/ → 运行期健康监控 → 退出时收进程组
//!
//! 这里刻意不给 pi-web 页面开任何 Tauri IPC：它是远程 origin，v1 只需要
//! 原生窗口/菜单/生命周期，不需要网页与 Rust 通信。

mod discovery;
mod logging;
mod menu;
mod probe;
mod progress;
mod quit_guard;
mod sidecar;
mod tray;
mod update;
mod window_state;
mod zoom;

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{Emitter, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Starting,
    Spawned,
    Attached,
    Failed,
}

pub struct AppState {
    pub mode: Mutex<Mode>,
    /// 允许导航到的服务端口（附着模式会变成对方端口）
    pub allowed_port: Arc<Mutex<Option<u16>>>,
    /// 当前服务地址（菜单"在浏览器中打开 / 复制地址"用）
    pub current_url: Arc<Mutex<Option<String>>>,
    pub sidecar: Mutex<Option<sidecar::Running>>,
    /// 最后一次状态：splash 页面加载可能晚于事件发出，加载后主动拉一次
    pub last_status: Mutex<Option<StatusPayload>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: Mutex::new(Mode::Starting),
            allowed_port: Arc::new(Mutex::new(None)),
            current_url: Arc::new(Mutex::new(None)),
            sidecar: Mutex::new(None),
            last_status: Mutex::new(None),
        }
    }
}

#[derive(Serialize, Clone)]
struct StatusPayload {
    phase: String,
    message: String,
    detail: Option<String>,
    url: Option<String>,
    port: Option<u16>,
    version: Option<String>,
    mode: Option<String>,
    tail: Vec<String>,
}

fn emit_status(app: &tauri::AppHandle, payload: StatusPayload) {
    if let Ok(mut slot) = app.state::<AppState>().last_status.lock() {
        *slot = Some(payload.clone());
    }
    let _ = app.emit("sidecar-status", payload);
}

fn status(app: &tauri::AppHandle, phase: &str, message: impl Into<String>) {
    let message = message.into();
    logging::line("app", &format!("[{phase}] {message}"));
    emit_status(
        app,
        StatusPayload {
            phase: phase.to_string(),
            message,
            detail: None,
            url: None,
            port: None,
            version: None,
            mode: None,
            tail: Vec::new(),
        },
    );
}

fn status_failed(app: &tauri::AppHandle, message: impl Into<String>, detail: impl Into<String>) {
    let message = message.into();
    let detail = detail.into();
    logging::line("app", &format!("[failed] {message} / {detail}"));
    emit_status(
        app,
        StatusPayload {
            phase: "failed".to_string(),
            message,
            detail: Some(detail),
            url: None,
            port: None,
            version: None,
            mode: None,
            tail: logging::tail(30),
        },
    );
}

fn status_ready(app: &tauri::AppHandle, url: &str, port: u16, version: &str, mode: &str) {
    logging::line("app", &format!("[ready] {url}（{mode}）"));
    emit_status(
        app,
        StatusPayload {
            phase: "ready".to_string(),
            message: format!("已就绪：{url}"),
            detail: None,
            url: Some(url.to_string()),
            port: Some(port),
            version: Some(version.to_string()),
            mode: Some(mode.to_string()),
            tail: Vec::new(),
        },
    );
}

fn navigate_main(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|e| format!("服务地址不合法：{e}"))?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;
    window
        .navigate(parsed)
        .map_err(|e| format!("导航失败：{e}"))?;
    if let Ok(mut current) = app.state::<AppState>().current_url.lock() {
        *current = Some(url.to_string());
    }
    Ok(())
}

pub fn current_url(app: &tauri::AppHandle) -> Option<String> {
    app.state::<AppState>()
        .current_url
        .lock()
        .ok()
        .and_then(|u| u.clone())
}

fn url_allowed(allowed_port: &Arc<Mutex<Option<u16>>>, url: &Url) -> bool {
    if url.scheme() == "tauri" {
        return true;
    }
    if url.scheme() != "http" {
        return false;
    }
    let port = allowed_port.lock().ok().and_then(|p| *p);
    match (url.host_str(), port) {
        (Some("127.0.0.1") | Some("localhost"), Some(expected)) => url.port() == Some(expected),
        _ => false,
    }
}

/// 恢复上一次"没链接到底"的体验：直接问用户怎么办，而不是白屏。
fn decide_on_mismatch(app: &tauri::AppHandle, existing: &discovery::ExistingInstance, ours: &str) -> bool {
    // 自动化演练（headless）时不弹窗，默认「启动独立实例」
    if std::env::var("PIWEB_UPDATE_HEADLESS").ok().as_deref() == Some("1") {
        logging::line("app", "headless：版本不一致时不附着，直接起自带的实例");
        return false;
    }
    let message = format!(
        "检测到本机已在运行 pi-web v{}（{}），本应用内置的是 v{ours}。\n\n连接已有实例 = 与浏览器/命令行共享同一个服务；启动独立实例 = 用内置版本另起一个（可能与已有实例同时操作同一份会话）。",
        existing.version, existing.url
    );
    // blocking_show() 返回 bool：true = 第一个按钮（"连接已有实例"）
    app.dialog()
        .message(message)
        .title("Pi Web Desktop")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "连接已有实例".to_string(),
            "启动独立实例".to_string(),
        ))
        .blocking_show()
}

fn run_launch(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    if let Ok(mut mode) = state.mode.lock() {
        *mode = Mode::Starting;
    }
    status(&app, "starting", "正在准备运行时…");

    sidecar::cleanup_previous(&app);

    let sidecar = match sidecar::resolve(&app) {
        Ok(s) => s,
        Err(error) => {
            if let Ok(mut mode) = state.mode.lock() {
                *mode = Mode::Failed;
            }
            status_failed(&app, "找不到内置的 pi-web 运行时", error.clone());
            // 自动化测试 / CI 里不要弹窗（否则会跳到用户屏幕上，0.3.1-beta.2 踩过）
            if std::env::var("PIWEB_UPDATE_HEADLESS").ok().as_deref() == Some("1")
                || std::env::var("PIWEB_NO_DIALOG").ok().as_deref() == Some("1")
            {
                logging::line("app", "headless：启动失败不弹窗");
                app.exit(1);
                return;
            }
            // true = 第一个按钮「退出」
            if app
                .dialog()
                .message(error)
                .title("Pi Web Desktop 启动失败")
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "退出".to_string(),
                    "查看日志".to_string(),
                ))
                .blocking_show()
            {
                app.exit(1);
            } else {
                use tauri_plugin_opener::OpenerExt;
                if let Some(path) = logging::path() {
                    if let Some(dir) = path.parent() {
                        let _ = app.opener().open_path(dir.to_string_lossy().to_string(), None::<&str>);
                    }
                }
            }
            return;
        }
    };
    let config = sidecar::Config::load(&app);

    // 1) 附着同版本实例：避免两份服务同时写同一份会话文件
    if config.attach_policy != "never" {
        if let Some(existing) = discovery::find_matching(&sidecar.version) {
            status(&app, "attached", format!("已连接本机运行的 pi-web v{}（{}）", existing.version, existing.url));
            if let Ok(mut port) = state.allowed_port.lock() {
                *port = Some(existing.port);
            }
            if let Ok(mut mode) = state.mode.lock() {
                *mode = Mode::Attached;
            }
            status_ready(&app, &existing.url, existing.port, &existing.version, "attached");
            if let Err(error) = navigate_main(&app, &existing.url) {
                status_failed(&app, "连接已有实例失败", error);
            }
            return;
        }
        if let Some(foreign) = discovery::find_foreign(&sidecar.version) {
            if decide_on_mismatch(&app, &foreign, &sidecar.version) {
                logging::line("app", &format!("用户选择连接 v{} 实例", foreign.version));
                if let Ok(mut port) = state.allowed_port.lock() {
                    *port = Some(foreign.port);
                }
                if let Ok(mut mode) = state.mode.lock() {
                    *mode = Mode::Attached;
                }
                status_ready(&app, &foreign.url, foreign.port, &foreign.version, "attached");
                let _ = navigate_main(&app, &foreign.url);
                return;
            }
            logging::line("app", "用户选择启动独立实例");
        }
    }

    // 2) 起自己的侧车
    let port = sidecar::pick_port(config.port);
    if port != config.port {
        logging::line(
            "app",
            &format!("端口 {} 被占用，改用 {port} 并写入 config", config.port),
        );
        let mut updated = config.clone();
        updated.port = port;
        updated.save(&app);
    }

    status(&app, "spawning", format!("正在启动 pi-web v{}（端口 {port}）…", sidecar.version));
    let running = match sidecar::spawn(&sidecar, &config, port) {
        Ok(r) => r,
        Err(error) => {
            if let Ok(mut mode) = state.mode.lock() {
                *mode = Mode::Failed;
            }
            status_failed(&app, "启动侧车失败", error);
            return;
        }
    };
    let launcher_pid = running.launcher_pid;
    sidecar::write_own_state(&app, &running);
    if let Ok(mut stored) = state.sidecar.lock() {
        *stored = Some(running);
    }
    if let Ok(mut allowed) = state.allowed_port.lock() {
        *allowed = Some(port);
    }
    if let Ok(mut mode) = state.mode.lock() {
        *mode = Mode::Spawned;
    }

    // 3) 等就绪（launcher 已退出 = 起不来）
    let should_stop = || {
        let state = app.state::<AppState>();
        let mut guard = state.sidecar.lock().unwrap();
        match guard.as_mut() {
            Some(running) => !matches!(running.child.try_wait(), Ok(None)),
            None => true,
        }
    };
    let ready = probe::wait_ready(port, Duration::from_secs(90), &should_stop);
    match ready {
        probe::ReadyResult::Ready(elapsed) => {
            logging::line("app", &format!("服务就绪用时 {}ms（launcher pid {launcher_pid}）", elapsed.as_millis()));
            let url = format!("http://127.0.0.1:{port}/");
            status_ready(&app, &url, port, &sidecar.version, "spawned");
            if let Err(error) = navigate_main(&app, &url) {
                status_failed(&app, "导航失败", error);
            }
        }
        probe::ReadyResult::Timeout(elapsed) => {
            let detail = format!(
                "等了 {}s 仍没有 HTTP 应答。日志尾部见下；常见原因：端口被占、被杀软拦截、侧车文件不完整。",
                elapsed.as_secs()
            );
            if let Ok(mut mode) = state.mode.lock() {
                *mode = Mode::Failed;
            }
            status_failed(&app, "pi-web 启动超时", detail);
        }
        probe::ReadyResult::Stopped => {
            if let Ok(mut mode) = state.mode.lock() {
                *mode = Mode::Failed;
            }
            status_failed(&app, "pi-web 进程提前退出", "launcher 已退出，请看日志尾部。");
        }
    }
}

static HEALTH_MONITOR_STARTED: AtomicBool = AtomicBool::new(false);

/// 下一次启动强制自己起侧车（热更新验证用）：不要附着到外部实例，
/// 否则"探活成功"可能只是外部那个实例在应答，误判成新版本可用。

/// 运行期健康监控，全局只有一个。
///
/// 关键点：**每轮都重新读「当前端口 + 当前模式」**，不在启动时把端口捕获进闭包。
/// 早先的写法在附着外部实例后把端口写死成对方的端口；用户停掉 npx 实例、点了
/// "重启服务"、应用切到自己起的 30241 之后，旧监控线程仍在盯已经死掉的 30141，
/// 于是每隔几十秒弹一次"服务已停止响应"——就是这个 bug。
///
/// 另外：一次故障只告警一次（`alerted`），恢复后才重新武装，避免刷屏。
fn start_health_monitor(app: tauri::AppHandle) {
    if HEALTH_MONITOR_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let interval = std::env::var("PIWEB_HEALTH_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(10));
    // 便于自动化验证：不弹原生对话框，直接按"重启服务"处理
    let headless = std::env::var("PIWEB_HEALTH_HEADLESS").ok().as_deref() == Some("1");

    std::thread::spawn(move || {
        let mut failures: u32 = 0;
        let mut alerted = false;
        let mut alerted_port: Option<u16> = None;
        loop {
            std::thread::sleep(interval);

            let (port, mode) = {
                let state = app.state::<AppState>();
                let port = state.allowed_port.lock().ok().and_then(|p| *p);
                let mode = *state.mode.lock().unwrap();
                (port, mode)
            };

            let Some(port) = port else {
                failures = 0;
                alerted = false;
                continue;
            };
            // 正在启动 / 已经失败：不归监控管，等状态机自己走完
            if matches!(mode, Mode::Starting | Mode::Failed) {
                failures = 0;
                continue;
            }
            if probe::http_answers(port, Duration::from_millis(800)) {
                if alerted && alerted_port == Some(port) {
                    logging::line("health", &format!("端口 {port} 已恢复"));
                }
                failures = 0;
                alerted = false;
                alerted_port = None;
                continue;
            }

            failures += 1;
            logging::line("health", &format!("端口 {port} 无应答（连续 {failures} 次）"));
            if failures < 3 || alerted {
                continue;
            }
            alerted = true;
            alerted_port = Some(port);

            let attached = matches!(mode, Mode::Attached);
            let detail = if attached {
                format!(
                    "外部实例 127.0.0.1:{port} 已停止响应——通常是你把 npx pi-web 那个命令结束了。\
点「启动服务」会改用内置 pi-web 起一个自己的实例（端口见日志）。"
                )
            } else {
                format!(
                    "本地服务 127.0.0.1:{port} 已停止响应——可能崩溃，或被 `pi-web stop` 停掉了。\
点「重启服务」重新拉起。"
                )
            };
            status_failed(&app, "pi-web 服务已停止响应", detail);

            let restart = if headless {
                logging::line("health", "headless：自动重启");
                true
            } else {
                let (ok_label, cancel_label) = if attached {
                    ("启动服务", "忽略")
                } else {
                    ("重启服务", "忽略")
                };
                app.dialog()
                    .message(format!(
                        "pi-web 服务已停止响应（127.0.0.1:{port}）。{}",
                        if attached {
                            "外部实例可能已被你在终端里结束。"
                        } else {
                            "本地侧车可能崩溃或被 pi-web stop 停掉。"
                        }
                    ))
                    .title("Pi Web Desktop")
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        ok_label.to_string(),
                        cancel_label.to_string(),
                    ))
                    .blocking_show()
            };

            if restart {
                failures = 0;
                if let Err(error) = restart_sidecar_now(&app) {
                    logging::line("health", &format!("自动重启失败：{error}"));
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    });
}

/// 停掉当前侧车并清掉状态（重启 / 热更新 / 退出都走这里）。
pub(crate) fn stop_sidecar(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let running = state.sidecar.lock().ok().and_then(|mut guard| guard.take());
    if let Some(running) = running {
        sidecar::stop(running);
    }
    sidecar::clear_own_state(app);
    *state.allowed_port.lock().unwrap() = None;
}

pub fn restart_sidecar_now(app: &tauri::AppHandle) -> Result<(), String> {
    logging::line("app", "重启侧车");
    stop_sidecar(app);
    // 立刻把状态置回 Starting：否则调用方（热更新/健康监控）可能读到上一轮的 Failed
    if let Ok(mut mode) = app.state::<AppState>().mode.lock() {
        *mode = Mode::Starting;
    }
    let handle = app.clone();
    std::thread::spawn(move || run_launch(handle));
    Ok(())
}


/// 当前是否由本应用自己托管着侧车（热更新验证"真的起来了"）。
pub(crate) fn owns_running_sidecar(app: &tauri::AppHandle) -> bool {
    app.state::<AppState>()
        .sidecar
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false)
}

/// 当前侧车端口（热更新探活用）。
pub(crate) fn current_sidecar_port(app: &tauri::AppHandle) -> Option<u16> {
    app.state::<AppState>()
        .allowed_port
        .lock()
        .ok()
        .and_then(|port| *port)
}

/// 热更新切换前调用：停掉旧版本，避免文件占用。
pub(crate) fn stop_sidecar_for_update(app: &tauri::AppHandle) {
    stop_sidecar(app);
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_status(state: tauri::State<'_, AppState>) -> Option<StatusPayload> {
    state.last_status.lock().ok().and_then(|s| s.clone())
}

#[tauri::command]
fn restart_sidecar(app: tauri::AppHandle) -> Result<(), String> {
    restart_sidecar_now(&app)
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    stop_sidecar(&app);
    app.exit(0);
}

#[tauri::command]
fn open_logs(app: tauri::AppHandle) {
    use tauri_plugin_opener::OpenerExt;
    if let Some(path) = logging::path() {
        if let Some(dir) = path.parent() {
            let _ = app.opener().open_path(dir.to_string_lossy().to_string(), None::<&str>);
        }
    }
}

#[tauri::command]
fn open_in_browser(app: tauri::AppHandle) {
    use tauri_plugin_opener::OpenerExt;
    if let Some(url) = current_url(&app) {
        let _ = app.opener().open_url(url, None::<&str>);
    }
}

/// 修 pi-web 输入框的横向滚动条（F20，2026-11-03）。
///
/// 现象：窗口变窄或页面缩放非 100% 时，输入框里出现一条横向滚动条，压在占位文字上。
/// 根因：pi-web 的 `.chat-input-textarea` 内联了 `overflow:auto`，而 **WebKit（WKWebView）
///      里 textarea 的 placeholder 不换行**（Chromium 会换行）—— 只要输入框宽度小于
///      placeholder 的文本宽度，就产生几十~几百 px 的横向溢出，于是画出横向滚动条。
///      实测（缩放 150%，窗口 1470×1011）：`textarea sw=817 cw=545 overflow-x=auto`。
/// 手段：每个文档注入一条 CSS，关掉输入框的横向滚动（textarea 本来靠软换行，不需要横向滚动）。
/// 放在初始化脚本里，随每次导航生效；L3 热更新换掉 pi-web 后依然有效。
const PIWEB_CSS_FIX: &str = r#"
(function () {
  var css = '.chat-input-textarea{overflow-x:hidden!important}';
  function inject() {
    if (document.getElementById('piweb-desktop-css-fix')) return;
    var style = document.createElement('style');
    style.id = 'piweb-desktop-css-fix';
    style.textContent = css;
    (document.head || document.documentElement).appendChild(style);
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', inject);
  else inject();
  setTimeout(inject, 1000);
  setTimeout(inject, 3000);
})();
"#;

fn new_window_for(
    app: &tauri::AppHandle,
    url: Url,
    features: tauri::webview::NewWindowFeatures,
) -> Result<tauri::WebviewWindow<tauri::Wry>, String> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let label = format!("page-{}", COUNTER.fetch_add(1, Ordering::Relaxed) + 1);
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::External(url.clone()))
        .title(url.as_str())
        .inner_size(1100.0, 820.0)
        // 关键：必须沿用调用方 webview 的配置。macOS 的 WKUIDelegate.createNewPage
        // 要求新 webview 与发起方使用同一 configuration，否则 WebKit 直接抛
        // NSException（表现为整个 App abort）。之前没传 -> 点「完整历史」必崩。
        .window_features(features)
        .initialization_script(PIWEB_CSS_FIX)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(window)
}

fn downloads_destination(app: &tauri::AppHandle, url: &Url) -> std::path::PathBuf {
    let file_name = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("pi-web-download");
    let base = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join(file_name)
}

/// 真正退出前的收尾：窗口几何落盘 + 停侧车 + 清 run record。
fn shutdown(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        window_state::save(app, &window, true);
    }
    let state = app.state::<AppState>();
    let running = state.sidecar.lock().ok().and_then(|mut guard| guard.take());
    if let Some(running) = running {
        sidecar::stop(running);
    }
    sidecar::clear_own_state(app);
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_status,
            progress::get_update_progress,
            restart_sidecar,
            quit_app,
            open_logs,
            open_in_browser
        ])
        .on_menu_event(|app, event| menu::on_event(app, event))
        .setup(|app| {
            let handle = app.handle().clone();

            if let Ok(log_dir) = app.path().app_log_dir() {
                if let Ok(path) = logging::init(log_dir) {
                    logging::line("app", &format!("日志：{}", path.display()));
                }
            }
            logging::line(
                "app",
                &format!(
                    "启动 Pi Web Desktop {}（{} {}）",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            );

            // 菜单必须在 setup 里装，不能用 `.menu(|app| menu::build(app))`：
            // 那个回调在 setup **之前**执行，此时 tauri_utils::Env（path 相关 state）还没
            // manage()，任何 `app.path()` 都会 panic —— 0.2.9 启动崩溃就是这个原因
            // （F15 的关于面板要读内置版本 → sidecar::resolve → app.path()）。
            match menu::build(&handle) {
                Ok(menu) => {
                    if let Err(error) = app.set_menu(menu) {
                        logging::line("app", &format!("安装菜单失败：{error}"));
                    }
                }
                Err(error) => logging::line("app", &format!("构建菜单失败：{error}")),
            }

            // 取消 L3 后：把历史上热更新留下的 overlay 清掉，回到内置固定版本
            sidecar::cleanup_overlay(&handle);

            // 菜单栏托盘（F16）
            match tray::create(&handle) {
                Ok(()) => {}
                Err(error) => logging::line("tray", &format!("创建菜单栏托盘失败：{error}")),
            }

            let window_config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or_else(|| "tauri.conf.json 里缺少窗口配置".to_string())?;

            let allowed_port = app.state::<AppState>().allowed_port.clone();

            let nav_guard = allowed_port.clone();
            let new_window_app = handle.clone();
            let new_window_guard = allowed_port.clone();

            let window = WebviewWindowBuilder::from_config(&handle, &window_config)?
                .initialization_script(PIWEB_CSS_FIX)
                .on_navigation(move |url| url_allowed(&nav_guard, url))
                .on_new_window(move |url, features| {
                    if url_allowed(&new_window_guard, &url) {
                        match new_window_for(&new_window_app, url, features) {
                            Ok(window) => NewWindowResponse::Create { window },
                            Err(error) => {
                                logging::line("nav", &format!("开新窗口失败：{error}"));
                                NewWindowResponse::Deny
                            }
                        }
                    } else {
                        use tauri_plugin_opener::OpenerExt;
                        let _ = new_window_app.opener().open_url(url.as_str(), None::<&str>);
                        NewWindowResponse::Deny
                    }
                })
                .on_download(|webview, event| {
                    match event {
                        DownloadEvent::Requested { url, destination } => {
                            let app = webview.app_handle();
                            *destination = downloads_destination(app, &url);
                            logging::line("download", &format!("下载 {url} -> {}", destination.display()));
                            true
                        }
                        DownloadEvent::Finished { url, path, success } => {
                            logging::line(
                                "download",
                                &format!("下载结束 {url} success={success} path={path:?}"),
                            );
                            true
                        }
                        _ => true,
                    }
                })
                .build()?;

            // F19：恢复上次的位置/大小（在 show 之前，避免可见跳动），并挂上记忆钩子。
            window_state::restore(&handle, &window);
            window_state::watch(&handle, &window);

            // F16 行为 B：关闭窗口 = 隐藏到菜单栏，不退出（pi-web 后台任务继续跑）。
            // 真正退出只走「托盘/菜单 → 退出」或 ⌘Q。
            let window_for_close = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window_for_close.hide();
                    crate::logging::line("window", "关闭按钮：已隐藏到菜单栏（可从菜单栏恢复）");
                }
            });

            let _ = window.show();

            // 恢复上次的页面缩放（菜单「视图 → 放大/缩小/实际大小」改的那个）
            zoom::restore(&handle);
            start_health_monitor(handle.clone());
            // 更新检测：只检查应用本身（L3 已取消）；启动查一次 + 运行期定期查
            update::silent_app_check(&handle);
            update::start_periodic_check(&handle);
            // 演练钩子：启动 3 秒后自动跑一次「检查更新」，用于端到端验证 L2/L3
            if std::env::var("PIWEB_UPDATE_CHECK_ON_STARTUP").ok().as_deref() == Some("1") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    update::manual_check_all(&handle);
                });
            }
            // 托盘/关窗探针：PIWEB_TRAY_PROBE=1 → 3 秒后调用 window.close()（应被拦截成隐藏），
            // 打一行状态再退出。用来在不需要点击 UI 的环境里验证 F16 的关窗行为。
            if std::env::var("PIWEB_TRAY_PROBE").ok().as_deref() == Some("1") {
                let probe_handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    if let Some(window) = probe_handle.get_webview_window("main") {
                        let _ = window.close();
                        std::thread::sleep(std::time::Duration::from_millis(800));
                        let visible = window.is_visible().unwrap_or(true);
                        crate::logging::line(
                            "tray-probe",
                            &format!("close() 之后 visible={visible}（应为 false=已隐藏到菜单栏）"),
                        );
                    }
                    probe_handle.exit(0);
                });
            }
            // 通道探针（验证「正式版/测试版」通道选择）：设 PIWEB_CHANNEL_PROBE=beta|stable
            if std::env::var("PIWEB_CHANNEL_PROBE").is_ok() {
                let probe_handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    update::probe_channel(&probe_handle);
                });
            }
            // 窗口探针（scripts/smoke-app.mjs 用）：3 秒后打印几何、落盘、退出。
            // 用来在没有 GUI 自动化权限的环境里验证「App 能启动 + 窗口几何存取」，常规启动不受影响。
            if std::env::var("PIWEB_WINDOW_PROBE").ok().as_deref() == Some("1") {
                let probe_handle = app.handle().clone();
                // 探针延时可调：smoke-app 要留时间确认「侧车真的在服务」
                let delay = std::env::var("PIWEB_WINDOW_PROBE_DELAY_SECS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(3);
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(delay));
                    if let Some(window) = probe_handle.get_webview_window("main") {
                        window_state::probe_and_save(&probe_handle, &window);
                    }
                    probe_handle.exit(0);
                });
            }
            std::thread::spawn(move || run_launch(handle));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("初始化 Tauri 应用失败");

    app.run(|app, event| match event {
        // 窗口隐藏到菜单栏后，点 Dock 图标应该把窗口叫回来（macOS 的 reopen 事件）
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                crate::logging::line("tray", "Dock 图标唤回窗口");
            }
        }
        // 退出保护（F22）：任何退出（⌘Q / 菜单退出 / 托盘退出）都先确认。
        // 关窗按钮走的是 F16 的「隐藏到菜单栏」，不会到这里。
        RunEvent::ExitRequested { api, code, .. } => {
            if !quit_guard::on_exit_requested(app, &api, code) {
                // 已拦下，等用户在确认框里选择
                return;
            }
            shutdown(app);
        }
        RunEvent::Exit => shutdown(app),
        _ => {}
    });
}
