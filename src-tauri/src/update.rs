//! 应用更新（L2，Tauri updater）+ 更新通道（正式版 / 测试版）。
//!
//! 2026-11-04 起**取消了 L3（只换 pi-web 的热更新）**：pi-web 版本与壳版本一对一
//! 固定，随 App 一起发布（见 docs/PLAN-2026-11-04-drop-l3.md）。
//! 因此这里不再有「查 npm 上游 pi-web 版本」的逻辑——那种提示已经无法转化为可用的更新。
//!
//! 检查时机：
//!   · 启动后 20 秒静默检查一次（每次启动都查）
//!   · 运行期间每 6 小时再查一次（`PIWEB_PERIODIC_CHECK_SECS` 可覆盖，测试用）
//!   · 菜单 / 托盘「检查应用更新…」手动触发
//! 同一版本只提醒一次（`update-state.json` 的 `notifiedAppVersion`）。

use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};


#[derive(Debug, Clone, Default)]
pub struct BundledVersions {
    pub app: String,
    pub pi_web: String,
    pub pi_sdk: String,
    pub node: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckState {
    #[serde(default)]
    last_app_checked_at: Option<u64>,
    /// 已经提醒过的版本（同一版本只弹一次，避免每次启动都打扰）
    #[serde(default)]
    notified_app_version: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn state_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("update-state.json"))
}

fn read_state(app: &tauri::AppHandle) -> CheckState {
    state_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_state(app: &tauri::AppHandle, state: &CheckState) {
    let Some(path) = state_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, format!("{text}\n"));
    }
}

/// 内置版本：pi-web 版本来自侧车 manifest，pi SDK 版本来自它自己的 package.json。
pub fn bundled_versions(app: &tauri::AppHandle) -> BundledVersions {
    let mut versions = BundledVersions {
        app: env!("CARGO_PKG_VERSION").to_string(),
        ..Default::default()
    };
    let Ok(sidecar) = crate::sidecar::resolve(app) else {
        return versions;
    };
    versions.pi_web = sidecar.version.clone();

    if let Ok(text) = std::fs::read_to_string(sidecar.dir.join("manifest.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(node) = json.get("nodeVersionActual").and_then(|v| v.as_str()) {
                versions.node = node.trim_start_matches('v').to_string();
            }
        }
    }
    // 节点包布局：<app>/node_modules/@agegr/pi-web，@earendil-works/* 是它的兄弟目录
    let sdk_package = sidecar
        .pkg_dir
        .parent()
        .and_then(|scope| scope.parent())
        .map(|node_modules| {
            node_modules
                .join("@earendil-works")
                .join("pi-coding-agent")
                .join("package.json")
        });
    if let Some(text) = sdk_package
        .as_deref()
        .and_then(|path| std::fs::read_to_string(path).ok())
    {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(version) = json.get("version").and_then(|v| v.as_str()) {
                versions.pi_sdk = version.to_string();
            }
        }
    }
    versions
}

/// 供 hotupdate 复用。
pub fn compare_versions_public(a: &str, b: &str) -> std::cmp::Ordering {
    compare_versions(a, b)
}

/// 数值化比较 `1.10.0 > 1.9.0`。返回 std::cmp::Ordering。
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    // 优先按 semver 比较：这样预发布版本顺序才对（0.3.0-beta.2 < 0.3.0 < 0.3.1），
    // 通道切换的「降级」判断依赖这个。
    fn parse(value: &str) -> Option<semver::Version> {
        semver::Version::parse(value.trim().trim_start_matches('v')).ok()
    }
    if let (Some(left), Some(right)) = (parse(a), parse(b)) {
        return left.cmp(&right);
    }
    // 非标准版本号（历史遗留串）回落到数字分段比较
    fn parts(value: &str) -> Vec<u64> {
        value
            .trim_start_matches('v')
            .split(['.', '-', '+'])
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (left, right) = (parts(a), parts(b));
    for index in 0..left.len().max(right.len()) {
        let l = left.get(index).copied().unwrap_or(0);
        let r = right.get(index).copied().unwrap_or(0);
        match l.cmp(&r) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// 供菜单「复制服务地址」复用（原来只写日志）。
pub fn copy_to_clipboard_public(text: &str) -> Result<(), String> {
    copy_to_clipboard(text)
}

fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("pbcopy", &[])
    } else if cfg!(windows) {
        ("clip", &[])
    } else {
        ("xclip", &["-selection", "clipboard"])
    };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("无法调用 {program}：{error}"))?;
    use std::io::Write;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|error| format!("写入剪贴板失败：{error}"))?;
    }
    let _ = child.wait();
    Ok(())
}

// ---------------------------------------------------------------- L2：应用自更新

use tauri_plugin_updater::UpdaterExt;

// ---------------------------------------------------------------- 更新通道
// 方案见 docs/PLAN-2026-11-03-channels.md：
//   正式版 -> tauri.conf.json 里的默认端点（releases/latest，GitHub 自动忽略预发布）
//   测试版 -> 固定 tag `channel-beta` 的 Release 里那份 latest.json（CI 每次发 beta 时覆盖上传）

pub const CHANNEL_STABLE: &str = "stable";
pub const CHANNEL_BETA: &str = "beta";
const BETA_MANIFEST_URL: &str =
    "https://github.com/ZilongYang/pi-web-desktop/releases/download/channel-beta/latest.json";

fn normalize_channel(raw: &str) -> &'static str {
    if raw == CHANNEL_BETA {
        CHANNEL_BETA
    } else {
        CHANNEL_STABLE
    }
}

/// 当前通道（config.json 的 `update_channel`）。
pub fn current_channel(app: &tauri::AppHandle) -> &'static str {
    normalize_channel(&crate::sidecar::Config::load(app).update_channel)
}

pub fn channel_label(channel: &str) -> &'static str {
    if normalize_channel(channel) == CHANNEL_BETA {
        "测试版 beta"
    } else {
        "正式版"
    }
}

/// 通道对应的清单地址；None = 用 tauri.conf.json 的默认端点（正式版）。
fn endpoint_for(channel: &str) -> Option<&'static str> {
    if normalize_channel(channel) == CHANNEL_BETA {
        Some(BETA_MANIFEST_URL)
    } else {
        None
    }
}

/// 按通道构造 updater。
/// `allow_any_version = true` 时允许「降级」（通道切换用：默认比较器只认更高版本）。
fn build_updater(
    app: &tauri::AppHandle,
    allow_any_version: bool,
) -> Result<tauri_plugin_updater::Updater, String> {
    build_updater_for(app, current_channel(app), allow_any_version)
}

/// 按**指定**通道构造 updater（探针用；正常流程走 `build_updater` 读 config）。
fn build_updater_for(
    app: &tauri::AppHandle,
    channel: &str,
    allow_any_version: bool,
) -> Result<tauri_plugin_updater::Updater, String> {
    let mut builder = app.updater_builder();
    if let Some(endpoint) = endpoint_for(channel) {
        let url: tauri::Url = endpoint.parse().map_err(|error| format!("{error}"))?;
        builder = builder.endpoints(vec![url]).map_err(|error| error.to_string())?;
    }
    if allow_any_version {
        builder = builder.version_comparator(|current, remote| remote.version != current);
    }
    builder.build().map_err(|error| error.to_string())
}

fn check_app_update_sync(
    app: &tauri::AppHandle,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let updater = build_updater(app, false)?;
    tauri::async_runtime::block_on(async { updater.check().await.map_err(|error| error.to_string()) })
}

fn check_any_app_update_sync(
    app: &tauri::AppHandle,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let updater = build_updater(app, true)?;
    tauri::async_runtime::block_on(async { updater.check().await.map_err(|error| error.to_string()) })
}

/// 通道探针（`PIWEB_CHANNEL_PROBE=beta|stable` 时由 main.rs 调用）：
/// 按指定通道检查一次更新（**只查不装**），写日志后退出。
/// 用途：在不需要点菜单、也不会触发下载的环境里验证「端点选择 + 版本比对」。
pub fn probe_channel(app: &tauri::AppHandle) {
    let raw = std::env::var("PIWEB_CHANNEL_PROBE").unwrap_or_else(|_| CHANNEL_STABLE.to_string());
    let channel = normalize_channel(&raw);
    let label = channel_label(channel);
    let allow_any = std::env::var("PIWEB_CHANNEL_PROBE_ALLOW_DOWNGRADE").ok().as_deref() == Some("1");
    crate::logging::line(
        "update-probe",
        &format!("通道={label} 清单={:?} 允许降级={allow_any}", endpoint_for(channel)),
    );
    let result = build_updater_for(app, channel, allow_any).and_then(|updater| {
        tauri::async_runtime::block_on(async { updater.check().await.map_err(|e| e.to_string()) })
    });
    match result {
        Ok(Some(update)) => {
            let current = env!("CARGO_PKG_VERSION");
            let direction = if compare_versions(update.version.as_str(), current)
                == std::cmp::Ordering::Less
            {
                "降级"
            } else {
                "升级"
            };
            crate::logging::line(
                "update-probe",
                &format!("发现 v{}（当前 v{current}）-> {direction}", update.version),
            );
        }
        Ok(None) => crate::logging::line(
            "update-probe",
            &format!("已是最新（当前 v{}）", env!("CARGO_PKG_VERSION")),
        ),
        Err(error) => crate::logging::line("update-probe", &format!("检查失败：{error}")),
    }
    app.exit(0);
}

/// 切换更新通道（菜单「更新通道」用）。
///
/// 写配置后立刻按新通道检查一次：
/// - 目标版本 ≠ 当前版本（**包括降级**）→ 弹窗确认后走现有的下载进度窗 + 安装 + 重启；
/// - 版本相同 → 只提示已切换。
pub fn switch_channel(app: &tauri::AppHandle, channel: &str) {
    let channel = normalize_channel(channel);
    let label = channel_label(channel);
    let mut config = crate::sidecar::Config::load(app);
    if config.update_channel == channel {
        crate::logging::line("update", &format!("更新通道已是{label}，重新检查一次"));
    } else {
        config.update_channel = channel.to_string();
        config.save(app);
        crate::logging::line("update", &format!("更新通道切换为{label}"));
    }

    let handle = app.clone();
    std::thread::spawn(move || {
        let checked = check_any_app_update_sync(&handle);
        match checked {
            Ok(Some(update)) => {
                let current = env!("CARGO_PKG_VERSION").to_string();
                let target = update.version.clone();
                let downgrade = compare_versions(&target, &current) == std::cmp::Ordering::Less;
                crate::logging::line(
                    "update",
                    &format!(
                        "{}通道：{} -> {}{}",
                        label,
                        current,
                        target,
                        if downgrade { "（降级重装）" } else { "" }
                    ),
                );
                let accepted = if headless() {
                    true
                } else {
                    let hint = if downgrade {
                        "这个版本比当前低，会重新安装一版较旧的正式版（覆盖安装）。\n\n"
                    } else {
                        ""
                    };
                    handle
                        .dialog()
                        .message(format!(
                            "已切换到{label}。\n\n\n{hint}检测到 {label} 版本 v{target}（当前 v{current}）。\n\n                             会下载整个应用包（约 130MB，含内置 pi-web 与 Node）并覆盖安装，完成后自动重启。\n                             有正在跑的任务请先保存。",
                        ))
                        .title(format!("Pi Web Desktop · 切换到{label}"))
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "下载并安装".to_string(),
                            "只切换，先不装".to_string(),
                        ))
                        .blocking_show()
                };
                if accepted {
                    if let Err(error) = install_app_update(&handle, update) {
                        crate::logging::line("update", &format!("通道切换安装失败：{error}"));
                    }
                }
            }
            Ok(None) => {
                crate::logging::line("update", &format!("{label}通道已是最新"));
                if !headless() {
                    handle
                        .dialog()
                        .message(format!(
                            "已切换到{label}。\n\n当前 v{} 在这个通道上已是最新。",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .title(format!("Pi Web Desktop · 切换到{label}"))
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "好的".to_string(),
                            "关闭".to_string(),
                        ))
                        .blocking_show();
                }
            }
            Err(error) => {
                crate::logging::line("update", &format!("{label}通道检查失败：{error}"));
                if !headless() {
                    handle
                        .dialog()
                        .message(format!(
                            "已切换到{label}，但检查更新失败：\n\n{error}\n\n可以稍后再点「检查应用更新…」。",
                        ))
                        .title("Pi Web Desktop · 切换更新通道")
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "知道了".to_string(),
                            "关闭".to_string(),
                        ))
                        .blocking_show();
                }
            }
        }
    });
}

fn install_app_update(
    app: &tauri::AppHandle,
    update: tauri_plugin_updater::Update,
) -> Result<(), String> {
    let version = update.version.clone();
    crate::logging::line("update", &format!("开始下载应用更新 v{version}"));

    // 立刻给用户可见的进度窗口：此前只有日志，13 秒里界面毫无反应，用户体验很差
    crate::progress::open(app);
    crate::progress::emit(
        app,
        "downloading",
        serde_json::json!({
            "title": "正在下载应用更新",
            "label": format!("Pi Web Desktop v{version}"),
            "downloaded": 0,
            "total": null,
            "hint": "下载完成后会询问是否重启。",
        }),
    );

    let mut last_logged = 0u64;
    let mut last_emitted = std::time::Instant::now() - Duration::from_secs(5);
    // updater 的回调第一个参数是「这一块的长度」，不是累计值（踩过：进度条一直 0MB）
    let mut accumulated: u64 = 0;
    let app_for_chunk = app.clone();
    let version_for_chunk = version.clone();
    let result = tauri::async_runtime::block_on(update.download_and_install(
        move |chunk_length: usize, total: Option<u64>| {
            accumulated += chunk_length as u64;
            let downloaded = accumulated;
            if downloaded.saturating_sub(last_logged) >= 20 * 1024 * 1024 {
                last_logged = downloaded;
                let total_text = total
                    .map(|t| format!("{:.0}MB", t as f64 / 1024.0 / 1024.0))
                    .unwrap_or_else(|| "?".to_string());
                crate::logging::line(
                    "update",
                    &format!(
                        "下载中 {:.0}MB / {total_text}",
                        downloaded as f64 / 1024.0 / 1024.0
                    ),
                );
            }
            if last_emitted.elapsed() >= Duration::from_millis(200) {
                last_emitted = std::time::Instant::now();
                crate::progress::emit(
                    &app_for_chunk,
                    "downloading",
                    serde_json::json!({
                        "title": "正在下载应用更新",
                        "label": format!("Pi Web Desktop v{version_for_chunk}"),
                        "downloaded": downloaded,
                        "total": total,
                        "hint": "下载完成后会询问是否重启。",
                    }),
                );
            }
        },
        || {
            crate::logging::line("update", "下载完成，正在安装");
        },
    ));

    if let Err(error) = result {
        crate::progress::emit(
            app,
            "failed",
            serde_json::json!({ "title": "应用更新失败", "message": error.to_string() }),
        );
        std::thread::sleep(Duration::from_millis(600));
        crate::progress::close(app);
        return Err(error.to_string());
    }

    crate::progress::emit(
        app,
        "ready",
        serde_json::json!({ "title": "应用更新已安装", "hint": "即将询问是否重启应用。" }),
    );
    crate::logging::line("update", "安装完成，等待用户确认重启");
    std::thread::sleep(Duration::from_millis(700));
    crate::progress::close(app);

    let restart = if headless() {
        true
    } else {
        app.dialog()
            .message(format!(
                "新版本 v{version} 已安装完成，重启应用后生效。\n\n现在重启，还是下次启动时生效？"
            ))
            .title("Pi Web Desktop · 更新完成")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "立即重启".to_string(),
                "稍后".to_string(),
            ))
            .blocking_show()
    };

    if restart {
        crate::logging::line("update", "用户选择立即重启");
        app.restart();
    } else {
        crate::logging::line("update", "用户选择稍后重启（新版本已装好，下次启动生效）");
        let _ = app
            .dialog()
            .message(format!("v{version} 已安装，下次启动 Pi Web Desktop 时生效。"))
            .title("Pi Web Desktop")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "好的".to_string(),
                "关闭".to_string(),
            ))
            .blocking_show();
    }
    #[allow(unreachable_code)]
    Ok(())
}

fn update_state_touch_app(app: &tauri::AppHandle) {
    let mut state = read_state(app);
    state.last_app_checked_at = Some(now_secs());
    write_state(app, &state);
}

/// 有新版时的提醒（启动静默检查与定期检查共用）。
fn notify_app_update(handle: &tauri::AppHandle, update: tauri_plugin_updater::Update) {
    crate::logging::line("update", &format!("应用有新版本 v{}", update.version));
    // 同一版本只提醒一次：每次启动都查，但不用每次开机都弹同一个窗
    let already = read_state(handle).notified_app_version.as_deref() == Some(update.version.as_str());
    if already && !headless() {
        crate::logging::line("update", "这个版本已经提醒过，本次不再弹窗");
        return;
    }
    let mut state = read_state(handle);
    state.notified_app_version = Some(update.version.clone());
    write_state(handle, &state);

    let accepted = if headless() {
        true
    } else {
        handle
            .dialog()
            .message(format!(
                "应用有新版本 v{}（当前 v{}）。\n\n更新会下载整个应用包（含内置 pi-web 与 Node），完成后自动重启。",
                update.version,
                env!("CARGO_PKG_VERSION")
            ))
            .title("Pi Web Desktop · 应用更新")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "下载并安装".to_string(),
                "稍后".to_string(),
            ))
            .blocking_show()
    };
    if accepted {
        if let Err(error) = install_app_update(handle, update) {
            crate::logging::line("update", &format!("应用更新失败：{error}"));
        }
    }
}

/// 启动后静默检查应用更新（**每次启动都查**，有更新才提示；不自动安装）。
pub fn silent_app_check(app: &tauri::AppHandle) {
    update_state_touch_app(app);
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        match check_app_update_sync(&handle) {
            // 静默：没有新版只写日志，绝不弹窗
            Ok(Some(update)) => notify_app_update(&handle, update),
            Ok(None) => crate::logging::line("update", "应用已是最新"),
            Err(error) => crate::logging::line("update", &format!("应用更新检查失败：{error}")),
        }
    });
}

/// 运行期间的定期检查（默认 6 小时一次；沿用「同一版本只提醒一次」）。
/// `PIWEB_PERIODIC_CHECK_SECS` 可覆盖间隔（测试用，最小 30 秒）。
pub fn start_periodic_check(app: &tauri::AppHandle) {
    let interval = std::env::var("PIWEB_PERIODIC_CHECK_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(6 * 60 * 60)
        .max(30);
    let handle = app.clone();
    std::thread::spawn(move || {
        crate::logging::line(
            "update",
            &format!("运行期定期检查已启动（每 {interval} 秒检查一次应用更新）"),
        );
        loop {
            std::thread::sleep(Duration::from_secs(interval));
            crate::logging::line("update", "运行期定期检查：应用更新");
            match check_app_update_sync(&handle) {
                Ok(Some(update)) => notify_app_update(&handle, update),
                Ok(None) => crate::logging::line("update", "定期检查：应用已是最新"),
                Err(error) => {
                    crate::logging::line("update", &format!("定期检查失败：{error}"))
                }
            }
        }
    });
}

/// 自动化/冒烟测试里不弹原生对话框（`smoke-app.mjs` 会设 `PIWEB_UPDATE_HEADLESS=1`）。
pub(crate) fn headless() -> bool {
    std::env::var("PIWEB_UPDATE_HEADLESS").ok().as_deref() == Some("1")
}

/// 菜单 / 托盘「检查应用更新…」：只检查应用本身（L3 已取消，pi-web 随壳升级）。
pub fn manual_check_all(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        crate::logging::line("update", "手动检查更新");
        match check_app_update_sync(&handle) {
            Ok(Some(update)) => {
                update_state_touch_app(&handle);
                let accepted = if headless() {
                    crate::logging::line("update", "headless：自动安装应用更新");
                    true
                } else {
                    handle
                        .dialog()
                        .message(format!(
                            "应用有新版本 v{}（当前 v{}）。\n\n\
                             更新会下载整个应用包（含内置 pi-web 与 Node），完成后自动重启。",
                            update.version,
                            env!("CARGO_PKG_VERSION")
                        ))
                        .title("Pi Web Desktop · 检查应用更新")
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "下载并安装".to_string(),
                            "跳过".to_string(),
                        ))
                        .blocking_show()
                };
                if accepted {
                    if let Err(error) = install_app_update(&handle, update) {
                        crate::logging::line("update", &format!("应用更新失败：{error}"));
                    }
                }
            }
            Ok(None) => {
                crate::logging::line("update", "应用已是最新");
                // 手动检查必须给反馈（0.3.2-beta.1 的疏漏：删掉 L3 后这里变成"点了没反应"）
                if !headless() {
                    let bundled = bundled_versions(&handle);
                    let channel = crate::update::channel_label(current_channel(&handle));
                    handle
                        .dialog()
                        .message(format!(
                            "已是最新。\n\n当前版本 v{}（{}通道）\n内置 pi-web {} · pi SDK {} · Node {}",
                            env!("CARGO_PKG_VERSION"),
                            channel,
                            if bundled.pi_web.is_empty() { "?" } else { &bundled.pi_web },
                            if bundled.pi_sdk.is_empty() { "?" } else { &bundled.pi_sdk },
                            if bundled.node.is_empty() { "?" } else { &bundled.node },
                        ))
                        .title("Pi Web Desktop · 检查应用更新")
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "好的".to_string(),
                            "关闭".to_string(),
                        ))
                        .blocking_show();
                }
            }
            Err(error) => {
                crate::logging::line("update", &format!("应用更新检查失败：{error}"));
                if !headless() {
                    handle
                        .dialog()
                        .message(format!(
                            "检查失败：\n\n{error}\n\n可稍后重试；网络/代理问题详见日志（菜单「打开日志目录」）。",
                        ))
                        .title("Pi Web Desktop · 检查应用更新")
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "好的".to_string(),
                            "关闭".to_string(),
                        ))
                        .blocking_show();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_update_channel() {
        assert_eq!(normalize_channel("beta"), CHANNEL_BETA);
        assert_eq!(normalize_channel("stable"), CHANNEL_STABLE);
        // 脏值/旧配置一律回落到正式版，避免出现「未知通道」这种半挂状态
        assert_eq!(normalize_channel("dev"), CHANNEL_STABLE);
        assert_eq!(normalize_channel(""), CHANNEL_STABLE);
    }

    #[test]
    fn beta_channel_points_at_fixed_manifest() {
        assert_eq!(endpoint_for(CHANNEL_BETA), Some(BETA_MANIFEST_URL));
        assert_eq!(endpoint_for(CHANNEL_STABLE), None); // 正式版用 tauri.conf 的默认端点
        assert!(BETA_MANIFEST_URL.ends_with("/releases/download/channel-beta/latest.json"));
    }

    #[test]
    fn channel_switch_detects_downgrade() {
        // 通道切换时「目标版本更低」要能识别出来 —— 这是「允许降级重装」的判断依据
        assert_eq!(
            compare_versions("0.2.11", "0.3.0-beta.1"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_versions("0.3.0-beta.2", "0.3.0-beta.1"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("0.3.0", "0.3.0-beta.2"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn compares_versions_numerically() {
        assert_eq!(compare_versions("0.14.0", "0.13.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare_versions("1.10.0", "1.9.9"), std::cmp::Ordering::Greater);
        assert_eq!(compare_versions("0.13.0", "0.13.0"), std::cmp::Ordering::Equal);
        assert_eq!(compare_versions("v22.23.4", "22.22.3"), std::cmp::Ordering::Greater);
    }


}
