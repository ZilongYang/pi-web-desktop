//! L3：只更新 pi-web（不动 Node、不重装 app）。
//!
//! 数据来源是我们自己 GitHub Release 里的两个资产：
//!   - `piweb-sidecar.json`：清单（piWebVersion + 各平台的 url/sha256/bytes）
//!   - `piweb-app-<piWebVersion>-<target>.tar.gz`：只含 `app/`（pi-web + 它的 node_modules）
//!     + `manifest.json`，不含 runtime/node
//!
//! 安装位置：`<app_local_data_dir>/sidecar/`（例如
//! `~/Library/Application Support/com.zilong.piweb.desktop/sidecar/`）。
//! `sidecar::resolve` 会优先用它；Node 仍然用包内那份，所以「只换 pi-web」。
//!
//! 安全与可回滚：
//!   1. 下载后校验 SHA256，不匹配直接丢弃；
//!   2. 解压到 staging，校验目录结构（`app/node_modules/@agegr/pi-web/bin/pi-web.js`）；
//!   3. 切换时把旧的 `app/` 改名成 `.prev/`，切换后重启侧车并探活；
//!   4. 探活失败就自动回滚到 `.prev/` 再重启，并把失败原因写日志 + 弹窗。
//!
//! 测试钩子：`PIWEB_SIDECAR_MANIFEST_URL` 可以指向本地 HTTP 服务，便于端到端演练。

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/ZilongYang/pi-web-desktop/releases/latest/download/piweb-sidecar.json";
const FETCH_TIMEOUT: &str = "10";
const READY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
pub struct SidecarManifest {
    #[serde(rename = "piWebVersion")]
    pub pi_web_version: String,
    #[serde(rename = "nodeVersion", default)]
    pub node_version: Option<String>,
    #[serde(default)]
    pub assets: HashMap<String, Asset>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub bytes: Option<u64>,
}

fn target_key() -> &'static str {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "darwin-aarch64"
    } else if cfg!(target_os = "macos") {
        "darwin-x64"
    } else if cfg!(windows) && cfg!(target_arch = "aarch64") {
        "windows-aarch64"
    } else if cfg!(windows) {
        "windows-x64"
    } else if cfg!(target_arch = "aarch64") {
        "linux-aarch64"
    } else {
        "linux-x64"
    }
}

fn manifest_url() -> String {
    std::env::var("PIWEB_SIDECAR_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MANIFEST_URL.to_string())
}

/// `<app_local_data_dir>/sidecar`：运行时 overlay 的根（里面放 app/ + manifest.json）。
pub fn overlay_root(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_local_data_dir().ok().map(|dir| dir.join("sidecar"))
}

fn staging_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_local_data_dir()
        .ok()
        .map(|dir| dir.join("sidecar-staging"))
}

fn proxy_from_config(app: &tauri::AppHandle) -> Option<String> {
    crate::sidecar::Config::load(app).proxy
}

fn curl(args: &[&str], proxy: Option<&str>) -> Result<Vec<u8>, String> {
    let mut command = Command::new("curl");
    command
        .args(["-s", "-L", "--max-time", FETCH_TIMEOUT, "--fail"])
        .stdin(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
        command.args(["--proxy", proxy]);
    }
    command.args(args);
    let output = command.output().map_err(|error| format!("无法执行 curl：{error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("curl 失败：{}", stderr.lines().next().unwrap_or("未知错误")));
    }
    Ok(output.stdout)
}

fn fetch_manifest(proxy: Option<&str>) -> Result<SidecarManifest, String> {
    let url = manifest_url();
    let body = curl(&["-H", "Accept: application/json", &url], proxy)?;
    serde_json::from_slice(&body).map_err(|error| format!("清单不是合法 JSON：{error}"))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|error| format!("打不开 {path:?}：{error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 256];
    loop {
        let read = file.read(&mut buffer).map_err(|error| format!("读取失败：{error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn human(bytes: u64) -> String {
    if bytes > 1024 * 1024 {
        format!("{:.0}MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{}KB", bytes / 1024)
    }
}

fn download_asset(app: &tauri::AppHandle, asset: &Asset) -> Result<PathBuf, String> {
    let staging = staging_dir(app).ok_or("拿不到 app local data 目录")?;
    std::fs::create_dir_all(&staging).map_err(|error| format!("创建 staging 失败：{error}"))?;
    let target = staging.join("sidecar.tar.gz");
    let _ = std::fs::remove_file(&target);
    let target_text = target.to_string_lossy().to_string();
    crate::logging::line("hotupdate", &format!("下载 {}", asset.url));
    curl(
        &["-o", &target_text, &asset.url],
        proxy_from_config(app).as_deref(),
    )?;
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    crate::logging::line("hotupdate", &format!("下载完成 {}", human(size)));
    if let Some(expected) = asset.bytes {
        if size != expected {
            return Err(format!("大小不符：期望 {expected}，实际 {size}"));
        }
    }
    let digest = sha256_file(&target)?;
    if !digest.eq_ignore_ascii_case(asset.sha256.trim()) {
        return Err(format!(
            "SHA256 校验失败：期望 {}，实际 {digest}",
            asset.sha256.trim()
        ));
    }
    crate::logging::line("hotupdate", "SHA256 校验通过");
    Ok(target)
}

fn extract_archive(app: &tauri::AppHandle, archive: &Path) -> Result<PathBuf, String> {
    let staging = staging_dir(app).ok_or("拿不到 app local data 目录")?;
    let unpacked = staging.join("unpacked");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|error| format!("创建解压目录失败：{error}"))?;

    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(&unpacked)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map_err(|error| format!("无法执行 tar：{error}"))?;
    if !status.success() {
        return Err("解压失败（tar 退出码非 0）".to_string());
    }
    Ok(unpacked)
}

fn validate_unpacked(unpacked: &Path) -> Result<(), String> {
    let entry = unpacked
        .join("app")
        .join("node_modules")
        .join("@agegr")
        .join("pi-web")
        .join("bin")
        .join("pi-web.js");
    if !entry.is_file() {
        return Err(format!("包里缺少 {}（不是侧车包？）", entry.display()));
    }
    if !unpacked.join("manifest.json").is_file() {
        return Err("包里缺少 manifest.json".to_string());
    }
    Ok(())
}

/// 原子切换：`sidecar/app` → `sidecar/.prev`，`unpacked/app` → `sidecar/app`。
fn swap_in(app: &tauri::AppHandle, unpacked: &Path) -> Result<PathBuf, String> {
    let root = overlay_root(app).ok_or("拿不到 app local data 目录")?;
    std::fs::create_dir_all(&root).map_err(|error| format!("创建 overlay 目录失败：{error}"))?;
    let current = root.join("app");
    let previous = root.join(".prev");
    let _ = std::fs::remove_dir_all(&previous);

    if current.is_dir() {
        std::fs::rename(&current, &previous)
            .map_err(|error| format!("备份旧版本失败（可能有文件占用）：{error}"))?;
    }
    if let Err(error) = std::fs::rename(unpacked.join("app"), &current) {
        // 尽量把旧的换回去，避免留下半成品
        let _ = std::fs::rename(&previous, &current);
        return Err(format!("切换新版本失败：{error}"));
    }
    if let Ok(manifest) = std::fs::read(unpacked.join("manifest.json")) {
        let _ = std::fs::write(root.join("manifest.json"), manifest);
    }
    crate::logging::line("hotupdate", &format!("已切换到新版本（备份在 {}）", previous.display()));
    Ok(previous)
}

fn rollback(app: &tauri::AppHandle, previous: &Path) -> Result<(), String> {
    let root = overlay_root(app).ok_or("拿不到 overlay 目录")?;
    let current = root.join("app");
    let broken = root.join("app.broken");
    let _ = std::fs::remove_dir_all(&broken);
    if current.exists() {
        let _ = std::fs::rename(&current, &broken);
    }
    if previous.exists() {
        std::fs::rename(previous, &current).map_err(|error| format!("回滚失败：{error}"))?;
        crate::logging::line("hotupdate", "已回滚到旧版本");
    }
    let _ = std::fs::remove_dir_all(&broken);
    Ok(())
}

fn cleanup_staging(app: &tauri::AppHandle) {
    if let Some(staging) = staging_dir(app) {
        let _ = std::fs::remove_dir_all(staging);
    }
}

/// 启动时调用：上次切换如果崩在半路，`.prev` 就是唯一完好的副本。
pub fn recover_on_startup(app: &tauri::AppHandle) {
    let Some(root) = overlay_root(app) else { return };
    let previous = root.join(".prev");
    let current = root.join("app");
    let current_ok = current
        .join("node_modules/@agegr/pi-web/bin/pi-web.js")
        .is_file();
    if previous.is_dir() && !current_ok {
        crate::logging::line("hotupdate", "检测到不完整的 overlay，恢复 .prev");
        let broken = root.join("app.broken");
        let _ = std::fs::remove_dir_all(&broken);
        if current.exists() {
            let _ = std::fs::rename(&current, &broken);
        }
        let _ = std::fs::rename(&previous, &current);
        let _ = std::fs::remove_dir_all(&broken);
    }
}

fn wait_sidecar_ready(app: &tauri::AppHandle, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        let port = crate::current_sidecar_port(app);
        if let Some(port) = port {
            if crate::probe::http_answers(port, Duration::from_millis(600)) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

/// 手动检查 + 安装（菜单触发）。
pub fn manual_check(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let proxy = proxy_from_config(&handle);
        let manifest = match fetch_manifest(proxy.as_deref()) {
            Ok(manifest) => manifest,
            Err(error) => {
                crate::logging::line("hotupdate", &format!("取清单失败：{error}"));
                show_dialog(
                    &handle,
                    &format!("检查 pi-web 更新失败：{error}\n\n（离线或还没发过 Release 时属正常）"),
                    false,
                );
                return;
            }
        };
        let current = crate::update::bundled_versions(&handle).pi_web;
        crate::logging::line(
            "hotupdate",
            &format!("清单里最新 pi-web {}（当前 {current}）", manifest.pi_web_version),
        );

        let asset = manifest.assets.get(target_key()).cloned();
        let newer = crate::update::compare_versions_public(&manifest.pi_web_version, &current)
            == std::cmp::Ordering::Greater;

        if !newer {
            show_dialog(
                &handle,
                &format!(
                    "pi-web 已是最新（{current}）。\n\n侧车包清单里的版本：{}",
                    manifest.pi_web_version
                ),
                false,
            );
            return;
        }
        let Some(asset) = asset else {
            show_dialog(
                &handle,
                &format!(
                    "pi-web {} 可用，但没有当前平台（{}）的侧车包。\n\n可以在 Release 里手动下载新版 app。",
                    manifest.pi_web_version,
                    target_key()
                ),
                false,
            );
            return;
        };

        let confirmed = if std::env::var("PIWEB_UPDATE_HEADLESS").ok().as_deref() == Some("1") {
            crate::logging::line("hotupdate", "headless：自动确认下载");
            true
        } else {
            handle
            .dialog()
            .message(format!(
                "可以把内置 pi-web 从 {current} 更新到 {}（只换 pi-web，不动 Node）。\n\n\
                 下载约 {}，切换后会自动重启本地服务；失败会自动回滚。",
                manifest.pi_web_version,
                asset.bytes.map(human).unwrap_or_else(|| "未知大小".to_string())
            ))
            .title("Pi Web Desktop · 更新 pi-web")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "下载并切换".to_string(),
                "取消".to_string(),
            ))
            .blocking_show()
        };
        if !confirmed {
            crate::logging::line("hotupdate", "用户取消");
            return;
        }

        match install(&handle, &asset) {
            Ok(()) => show_dialog(
                &handle,
                &format!("已更新到 pi-web {}，服务已重启。", manifest.pi_web_version),
                false,
            ),
            Err(error) => {
                crate::logging::line("hotupdate", &format!("安装失败：{error}"));
                show_dialog(&handle, &format!("更新失败：{error}\n\n已保持/回滚到原版本。"), false);
            }
        }
    });
}

fn install(app: &tauri::AppHandle, asset: &Asset) -> Result<(), String> {
    cleanup_staging(app);
    let archive = download_asset(app, asset)?;
    let unpacked = extract_archive(app, &archive)?;
    validate_unpacked(&unpacked)?;

    // 停服务 → 换目录 → 重启 → 探活
    crate::stop_sidecar_for_update(app);
    let previous = swap_in(app, &unpacked)?;
    crate::restart_sidecar_now(app)?;
    if wait_sidecar_ready(app, READY_TIMEOUT) {
        let _ = std::fs::remove_dir_all(&previous);
        cleanup_staging(app);
        crate::logging::line("hotupdate", "新版本侧车启动正常，已删除旧版本备份");
        return Ok(());
    }

    // 起不来：回滚
    crate::logging::line("hotupdate", "新版本侧车未就绪，开始回滚");
    crate::stop_sidecar_for_update(app);
    rollback(app, &previous)?;
    crate::restart_sidecar_now(app)?;
    let recovered = wait_sidecar_ready(app, READY_TIMEOUT);
    cleanup_staging(app);
    if recovered {
        Err("新版本启动失败，已回滚并恢复旧版本".to_string())
    } else {
        Err("新版本启动失败，回滚后旧版本也没起来 —— 请查看日志或重启应用".to_string())
    }
}

fn show_dialog(app: &tauri::AppHandle, message: &str, can_copy: bool) {
    crate::logging::line("hotupdate", &format!("提示：{message}"));
    if std::env::var("PIWEB_UPDATE_HEADLESS").ok().as_deref() == Some("1") {
        return;
    }
    let _ = can_copy;
    app.dialog()
        .message(message.to_string())
        .title("Pi Web Desktop · 更新 pi-web")
        .buttons(MessageDialogButtons::OkCancelCustom("好的".to_string(), "关闭".to_string()))
        .blocking_show();
}
