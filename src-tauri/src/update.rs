//! 更新检测（L1）：只查、只提示，不下载、不替换。
//!
//! 为什么不做自动升级：pi-web / Node 都打包在已签名的 `.app` 里，替换要走
//! Tauri updater（换整个 app，需要发布源 + 签名）或运行时侧车热更新（需要
//! 侧车压缩包 + SHA256 校验 + 原子切换）。在那两层没搭起来之前，任何"自动下载"
//! 都等于在用户机器上执行来历不明的代码，所以这里只做到提示 + 一键复制升级命令。
//!
//! 数据源用 npm registry 的 `@agegr%2Fpi-web/latest` —— pi-web 自己的
//! `/api/app-update` 也是查这个（12h 缓存）。本应用注入了
//! `PI_WEB_SKIP_VERSION_CHECK=1` 关掉它自带的提示，因为它给的更新方式是
//! `npm install -g pi-web`，对内置副本无效；提示语气与动作由壳统一负责。

use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

const REGISTRY_URL: &str = "https://registry.npmjs.org/@agegr%2Fpi-web/latest";
const FETCH_TIMEOUT_SECS: &str = "8";

/// 升级命令：放在提示里可一键复制。
const UPGRADE_COMMAND: &str = "cd ~/zWork/ai/pi-web-desktop && npm run sidecar && npm run build";

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
    last_checked_at: Option<u64>,
    #[serde(default)]
    last_app_checked_at: Option<u64>,
    /// 已经提醒过的版本（同一版本只弹一次，避免每次启动都打扰）
    #[serde(default)]
    notified_app_version: Option<String>,
    #[serde(default)]
    notified_piweb_version: Option<String>,
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

/// 用 curl 查 npm registry（GUI 进程没有 shell，也不需要额外的 TLS 依赖）。
/// 系统 curl 在 macOS 与 Windows 10+ 都有；配置了代理就带上 `--proxy`。
fn fetch_latest_pi_web(proxy: Option<&str>) -> Result<String, String> {
    // 测试钩子：可以假装 registry 上是某个版本，用来验证"有新版"的提示文案
    if let Ok(fake) = std::env::var("PIWEB_UPDATE_FAKE_LATEST") {
        if !fake.trim().is_empty() {
            crate::logging::line("update", &format!("（测试钩子）假装最新版本为 {fake}"));
            return Ok(fake.trim().to_string());
        }
    }
    let mut command = Command::new("curl");
    command
        .args(["-s", "--max-time", FETCH_TIMEOUT_SECS, "--fail", "-H", "Accept: application/json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
        command.args(["--proxy", proxy]);
    }
    command.arg(REGISTRY_URL);

    let output = command.output().map_err(|error| format!("无法执行 curl：{error}"))?;
    if !output.status.success() {
        return Err(format!("curl 退出码 {:?}", output.status.code()));
    }
    let body: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("registry 返回的不是 JSON：{error}"))?;
    body.get("version")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .ok_or_else(|| "registry 返回里没有 version 字段".to_string())
}

fn upgrade_available(bundled: &BundledVersions, latest: &str) -> bool {
    !bundled.pi_web.is_empty()
        && !latest.is_empty()
        && compare_versions(latest, &bundled.pi_web) == std::cmp::Ordering::Greater
}

fn build_message(bundled: &BundledVersions, latest: &Result<String, String>) -> (String, bool) {
    let header = format!(
        "内置 Pi Web Desktop {} · pi-web {} · pi SDK {} · Node {}",
        bundled.app,
        if bundled.pi_web.is_empty() { "?" } else { &bundled.pi_web },
        if bundled.pi_sdk.is_empty() { "?" } else { &bundled.pi_sdk },
        if bundled.node.is_empty() { "?" } else { &bundled.node },
    );

    match latest {
        Ok(latest) if upgrade_available(bundled, latest) => (
            format!(
                "检测到新版 pi-web：{latest}（内置 {}）。\n\n{header}\n\n\
                 pi SDK 由 pi-web 精确锁定，会跟着一起升级。\n\n\
                 想现在就用上：菜单「Pi Web → 更新 pi-web…」，可以直接下载并切换 pi-web（只换 pi-web，不重装 App）。\n\
                 或者重新构建整个 App（约 2 分钟）：\n\n\
                 {UPGRADE_COMMAND}",
                bundled.pi_web
            ),
            true,
        ),
        Ok(latest) => (
            format!("已是最新。\n\n{header}\n\nnpm registry 上最新的 pi-web 是 {latest}。"),
            false,
        ),
        Err(error) => (
            format!("检查失败：{error}\n\n{header}\n\n（离线或被墙时属正常，可稍后再试）"),
            false,
        ),
    }
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

fn show_dialog(app: &tauri::AppHandle, message: &str, can_copy: bool) {
    crate::logging::line("update", &format!("提示内容：{message}"));
    if std::env::var("PIWEB_UPDATE_HEADLESS").ok().as_deref() == Some("1") {
        return;
    }
    let copy = app
        .dialog()
        .message(message.to_string())
        .title("Pi Web Desktop · 检查 pi-web 更新")
        .buttons(if can_copy {
            MessageDialogButtons::OkCancelCustom("复制升级命令".to_string(), "知道了".to_string())
        } else {
            MessageDialogButtons::OkCancelCustom("好的".to_string(), "关闭".to_string())
        })
        .blocking_show();

    if can_copy && copy {
        match copy_to_clipboard(UPGRADE_COMMAND) {
            Ok(()) => crate::logging::line("update", "升级命令已复制到剪贴板"),
            Err(error) => crate::logging::line("update", &format!("复制失败：{error}")),
        }
    }
}

/// 手动检查（菜单触发）：不节流，总是弹结果。
pub fn manual_check(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let bundled = bundled_versions(&handle);
        let proxy = crate::sidecar::Config::load(&handle).proxy;
        crate::logging::line(
            "update",
            &format!("手动检查更新（内置 pi-web {}）", bundled.pi_web),
        );
        let latest = fetch_latest_pi_web(proxy.as_deref());
        if let Ok(version) = latest.as_ref() {
            let mut state = read_state(&handle);
            state.last_checked_at = Some(now_secs());
            write_state(&handle, &state);
            crate::logging::line("update", &format!("npm 最新 pi-web：{version}"));
        }
        let (message, can_copy) = build_message(&bundled, &latest);
        show_dialog(&handle, &message, can_copy);
    });
}

/// 启动时的静默检查（npm 上的 pi-web）：**每次启动都查**；有新版才提示，失败不打扰。
pub fn silent_check(app: &tauri::AppHandle) {
    {
        let mut state = read_state(app);
        state.last_checked_at = Some(now_secs());
        write_state(app, &state);
    }

    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        let bundled = bundled_versions(&handle);
        let proxy = crate::sidecar::Config::load(&handle).proxy;
        let latest = fetch_latest_pi_web(proxy.as_deref());
        match latest {
            Ok(version) if upgrade_available(&bundled, &version) => {
                crate::logging::line(
                    "update",
                    &format!("发现新版 pi-web {version}（内置 {}）", bundled.pi_web),
                );
                let already = read_state(&handle)
                    .notified_piweb_version
                    .as_deref()
                    == Some(version.as_str());
                if already && !headless() {
                    crate::logging::line("update", "这个 pi-web 版本已经提醒过，本次不再弹窗");
                    return;
                }
                let mut state = read_state(&handle);
                state.notified_piweb_version = Some(version.clone());
                write_state(&handle, &state);
                let (message, can_copy) = build_message(&bundled, &Ok(version));
                show_dialog(&handle, &message, can_copy);
            }
            Ok(version) => crate::logging::line("update", &format!("已是最新（{version}）")),
            Err(error) => crate::logging::line("update", &format!("静默检查失败：{error}")),
        }
    });
}

// ---------------------------------------------------------------- L2：应用自更新

use tauri::{Emitter, WebviewUrl, WebviewWindowBuilder};
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

/// 启动后静默检查应用更新（**每次启动都查**，有更新才提示；不自动安装）。
pub fn silent_app_check(app: &tauri::AppHandle) {
    update_state_touch_app(app);
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        match check_app_update_sync(&handle) {
            Ok(Some(update)) => {
                crate::logging::line("update", &format!("应用有新版本 v{}", update.version));
                // 同一版本只提醒一次：每次启动都查，但不用每次开机都弹同一个窗
                let already = read_state(&handle)
                    .notified_app_version
                    .as_deref()
                    == Some(update.version.as_str());
                if already && !headless() {
                    crate::logging::line("update", "这个版本已经提醒过，本次不再弹窗");
                    return;
                }
                let mut state = read_state(&handle);
                state.notified_app_version = Some(update.version.clone());
                write_state(&handle, &state);
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
                    if let Err(error) = install_app_update(&handle, update) {
                        crate::logging::line("update", &format!("应用更新失败：{error}"));
                    }
                }
            }
            Ok(None) => crate::logging::line("update", "应用已是最新"),
            Err(error) => crate::logging::line("update", &format!("应用更新检查失败：{error}")),
        }
    });
}

fn headless() -> bool {
    std::env::var("PIWEB_UPDATE_HEADLESS").ok().as_deref() == Some("1")
}

/// 菜单「检查应用更新…」的总入口：先看应用本身，再看能热更新的 pi-web
/// （F17：菜单文案与弹窗都要讲清楚这一步只处理应用更新）。
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
                             这里只处理应用本身的更新；跳过或已是最新时，会继续检查 pi-web 更新。\n\n\
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
                        crate::logging::line("update", "继续检查 pi-web 更新");
                        crate::hotupdate::manual_check(&handle);
                    }
                    return;
                }
                // 用户跳过了应用更新 → 继续查 pi-web（这样 L3 可以单独验证/使用）
                crate::logging::line("update", "用户跳过应用更新，继续检查 pi-web");
            }
            Ok(None) => crate::logging::line("update", "应用已是最新"),
            Err(error) => crate::logging::line(
                "update",
                &format!("应用更新检查失败（{error}），继续检查 pi-web"),
            ),
        }
        crate::hotupdate::manual_check(&handle);
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

    #[test]
    fn detects_upgrade_only_when_strictly_newer() {
        let bundled = BundledVersions {
            app: "0.1.0".into(),
            pi_web: "0.13.0".into(),
            ..Default::default()
        };
        assert!(!upgrade_available(&bundled, "0.13.0"));
        assert!(upgrade_available(&bundled, "0.13.1"));
        assert!(!upgrade_available(&bundled, "0.12.9"));
    }

    #[test]
    fn message_says_up_to_date_when_equal() {
        let bundled = BundledVersions {
            app: "0.1.0".into(),
            pi_web: "0.13.0".into(),
            pi_sdk: "1.4.0".into(),
            node: "22.23.4".into(),
        };
        let (message, can_copy) = build_message(&bundled, &Ok("0.13.0".to_string()));
        assert!(!can_copy);
        assert!(message.contains("已是最新"));
        assert!(message.contains("Node 22.23.4"));
    }

    #[test]
    fn message_offers_command_when_newer() {
        let bundled = BundledVersions {
            app: "0.1.0".into(),
            pi_web: "0.13.0".into(),
            pi_sdk: "1.4.0".into(),
            node: "22.23.4".into(),
        };
        let (message, can_copy) = build_message(&bundled, &Ok("0.14.0".to_string()));
        assert!(can_copy);
        assert!(message.contains("npm run sidecar"));
    }
}
