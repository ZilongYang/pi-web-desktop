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
const SILENT_INTERVAL_SECS: u64 = 24 * 3600;
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
                 pi SDK 由 pi-web 精确锁定，会跟着一起升级。\n\
                 桌面版把 pi-web 打包在 .app 里，升级需要重新构建（约 2 分钟）：\n\n\
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
        .title("Pi Web Desktop · 检查更新")
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

/// 启动时的静默检查：24h 一次；有新版才提示，没新版/失败都不打扰。
pub fn silent_check(app: &tauri::AppHandle) {
    let state = read_state(app);
    if let Some(last) = state.last_checked_at {
        if now_secs().saturating_sub(last) < SILENT_INTERVAL_SECS {
            crate::logging::line("update", "静默检查跳过（24h 内已查过）");
            return;
        }
    }
    // 先把时间戳写掉：离线时也不会每次启动都重试
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
                let (message, can_copy) = build_message(&bundled, &Ok(version));
                show_dialog(&handle, &message, can_copy);
            }
            Ok(version) => crate::logging::line("update", &format!("已是最新（{version}）")),
            Err(error) => crate::logging::line("update", &format!("静默检查失败：{error}")),
        }
    });
}

// ---------------------------------------------------------------- L2：应用自更新

use tauri_plugin_updater::UpdaterExt;

fn check_app_update_sync(
    app: &tauri::AppHandle,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let updater = app.updater().map_err(|error| error.to_string())?;
    tauri::async_runtime::block_on(async { updater.check().await.map_err(|error| error.to_string()) })
}

fn install_app_update(
    app: &tauri::AppHandle,
    update: tauri_plugin_updater::Update,
) -> Result<(), String> {
    let version = update.version.clone();
    crate::logging::line("update", &format!("开始下载应用更新 v{version}"));
    let mut last_logged = 0u64;
    tauri::async_runtime::block_on(update.download_and_install(
        |downloaded: usize, total: Option<u64>| {
            let downloaded = downloaded as u64;
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
        },
        || crate::logging::line("update", "下载完成，正在安装"),
    ))
    .map_err(|error| error.to_string())?;
    crate::logging::line("update", "安装完成，重启应用");
    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}

fn update_state_touch_app(app: &tauri::AppHandle) {
    let mut state = read_state(app);
    state.last_app_checked_at = Some(now_secs());
    write_state(app, &state);
}

/// 启动后静默检查应用更新（24h 一次，有更新才提示）。
pub fn silent_app_check(app: &tauri::AppHandle) {
    if let Some(last) = read_state(app).last_app_checked_at {
        if now_secs().saturating_sub(last) < SILENT_INTERVAL_SECS {
            return;
        }
    }
    update_state_touch_app(app);
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        match check_app_update_sync(&handle) {
            Ok(Some(update)) => {
                crate::logging::line("update", &format!("应用有新版本 v{}", update.version));
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

/// 菜单「检查更新…」的总入口：先看应用本身，再看能热更新的 pi-web。
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
                            "应用有新版本 v{}（当前 v{}）。\n\n更新会下载整个应用包（含内置 pi-web 与 Node），完成后自动重启。",
                            update.version,
                            env!("CARGO_PKG_VERSION")
                        ))
                        .title("Pi Web Desktop · 检查更新")
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
                }
                return;
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
