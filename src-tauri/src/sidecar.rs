//! 侧车生命周期：定位资源 → 选端口 → 起 `node bin/pi-web.js` → 收尾。
//!
//! 设计约束（来自对 pi-web 的实测与源码阅读）：
//! - 必须走官方 launcher `bin/pi-web.js`：它在启动前轮换 `previewModeId` 等密钥、
//!   校验 Node 版本、并把信号转发给真正的 `next start` 子进程。直接起 `next start`
//!   等于绕过这些安全与生命周期逻辑。
//! - launcher 自己会 spawn 一个子进程，所以收尾必须按「进程组」处理，否则 `next-server`
//!   会变成孤儿占着端口（Windows 只能用 taskkill /T）。
//! - 就绪判定用 `GET /`，不是解析 stdout 的 "Ready"。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tauri::Manager;

/// 不依赖 AppHandle 的配置目录（sidecar 模块在配置加载之外也要用）。
mod app_dirs {
    use std::path::PathBuf;

    pub fn config_dir() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("PIWEB_CONFIG_DIR") {
            return Some(PathBuf::from(dir));
        }
        #[cfg(target_os = "macos")]
        {
            let home = crate::discovery::home_dir()?;
            return Some(home.join("Library/Application Support/com.zilong.piweb.desktop"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let home = crate::discovery::home_dir()?;
            Some(home.join(".config/pi-web-desktop"))
        }
    }
}

pub const PORT_CANDIDATES: u16 = 10;
/// 自己的默认端口刻意与 pi-web CLI 的 30141 错开：否则桌面版先占了 30141，
/// 终端里再 `npx @agegr/pi-web` 就会撞端口。同版本实例在跑时走附着，不受影响。
pub const DEFAULT_PORT: u16 = 30241;

// ---------------------------------------------------------------- config

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    /// "same-version"（默认）| "never"
    #[serde(default = "default_attach_policy")]
    pub attach_policy: String,
    /// 例如 "http://127.0.0.1:7890"；None = 不注入代理（GUI 本来也拿不到终端里的代理变量）
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub extra_env: HashMap<String, String>,
    /// 默认不设，即共享 ~/.pi/agent
    #[serde(default)]
    pub pi_coding_agent_dir: Option<String>,
    /// 是否继承登录 shell 的 PATH（默认 true）。
    /// GUI 启动的 app 只有 /usr/bin:/bin:/usr/sbin:/sbin，nvm/pnpm/asdf 这些只在
    /// 交互式 shell 的 rc 里才会写进 PATH，所以这里跑一次 `zsh -lic` 取回来。
    #[serde(default = "default_true")]
    pub inherit_login_shell_path: bool,
    /// 页面缩放档位（1.0 = 100%），由菜单「视图」维护
    #[serde(default = "default_zoom")]
    pub zoom: f64,
    /// 主窗口上次的位置与大小（物理像素），由 `window_state` 在移动/缩放/关窗时维护
    #[serde(default)]
    pub window: Option<WindowGeometry>,
    /// 更新通道："stable"（默认）| "beta"。见 docs/PLAN-2026-11-03-channels.md
    #[serde(default = "default_update_channel")]
    pub update_channel: String,
}

fn default_update_channel() -> String {
    "stable".to_string()
}

/// 主窗口几何（物理像素）：`x/y` 是窗口外框左上角，`width/height` 是内容区尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

fn default_zoom() -> f64 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_port() -> u16 {
    DEFAULT_PORT
}
fn default_attach_policy() -> String {
    "same-version".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            attach_policy: default_attach_policy(),
            proxy: None,
            extra_env: HashMap::new(),
            pi_coding_agent_dir: None,
            inherit_login_shell_path: true,
            zoom: 1.0,
            window: None,
            update_channel: default_update_channel(),
        }
    }
}

fn config_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    // 测试/多配置用：PIWEB_CONFIG_DIR 指定配置目录。
    // scripts/smoke-app.mjs 靠它隔离，验证启动/窗口几何时不碰用户真实配置。
    if let Some(dir) = std::env::var_os("PIWEB_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("config.json"));
    }
    app.path().app_config_dir().ok().map(|d| d.join("config.json"))
}

impl Config {
    pub fn load(app: &tauri::AppHandle) -> Config {
        let Some(path) = config_path(app) else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(cfg) => cfg,
                Err(error) => {
                    crate::logging::line("config", &format!("{} 解析失败，用默认值: {error}", path.display()));
                    Config::default()
                }
            },
            Err(_) => {
                let cfg = Config::default();
                cfg.save(app);
                cfg
            }
        }
    }

    pub fn save(&self, app: &tauri::AppHandle) {
        let Some(path) = config_path(app) else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, format!("{text}\n"));
        }
    }
}

// ---------------------------------------------------------------- sidecar

#[derive(Debug, Clone)]
pub struct Sidecar {
    pub dir: PathBuf,
    pub node: PathBuf,
    pub pkg_dir: PathBuf,
    pub entry: PathBuf,
    pub version: String,
}

fn node_bin_name() -> &'static str {
    if cfg!(windows) {
        "node.exe"
    } else {
        "node"
    }
}

/// manifest.json 由 scripts/fetch-sidecar.mjs 生成；热更新的 overlay 也带一份。
pub fn read_sidecar_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("piWebVersion")?.as_str().map(|s| s.to_string())
}

/// 从"根目录 + node 路径"组装一个可用的侧车。
/// 根目录里必须有 `app/node_modules/@agegr/pi-web/bin/pi-web.js`；
/// 包内那份还额外有 `runtime/node`，运行时 overlay（L3）则复用包内的 node。
fn from_root(root: &Path, node: PathBuf) -> Option<Sidecar> {
    let pkg_dir = root
        .join("app")
        .join("node_modules")
        .join("@agegr")
        .join("pi-web");
    let entry = pkg_dir.join("bin").join("pi-web.js");
    if !node.is_file() || !entry.is_file() {
        return None;
    }
    let version = read_sidecar_version(root)
        .or_else(|| {
            let text = std::fs::read_to_string(pkg_dir.join("package.json")).ok()?;
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            json.get("version")?.as_str().map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    Some(Sidecar {
        dir: root.to_path_buf(),
        node,
        pkg_dir,
        entry,
        version,
    })
}

/// 开发目录兜底（仅 debug 构建）。release 里必须返回 None：`env!("CARGO_MANIFEST_DIR")`
/// 会把构建机的绝对路径嵌进二进制，等于把家目录公开出去（make-dist.sh 会断言这一点）。
#[cfg(debug_assertions)]
fn dev_sidecar_root() -> Option<PathBuf> {
    Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("sidecar"))
}

#[cfg(not(debug_assertions))]
fn dev_sidecar_root() -> Option<PathBuf> {
    None
}

fn bundled_node(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join("pi-web").join("runtime").join(node_bin_name());
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Some(dev_root) = dev_sidecar_root() {
        let dev = dev_root.join("runtime").join(node_bin_name());
        if dev.is_file() {
            return Some(dev);
        }
    }
    None
}

/// 解析当前该用哪份侧车，优先级：
///   1. 运行时热更新装上的 overlay（`<app_local_data_dir>/sidecar`，只含 app/ + manifest.json）
///   2. 包内 `Resources/pi-web`
///   3. dev 的仓库根 `sidecar/`
///
/// overlay 永远复用包内的 Node —— 这正是 L3"只换 pi-web、不动 Node"的意思。
pub fn resolve(app: &tauri::AppHandle) -> Result<Sidecar, String> {
    let mut tried: Vec<String> = Vec::new();

    // 先算出包内那份的版本，overlay 只有严格更新时才用（否则 app 升级到更高版本后
    // 还会被旧 overlay 顶住，等于"降级运行"）。
    let mut bundled_version = String::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        bundled_version = read_sidecar_version(&resource_dir.join("pi-web")).unwrap_or_default();
    }
    if bundled_version.is_empty() {
        if let Some(dev_root) = dev_sidecar_root() {
            bundled_version = read_sidecar_version(&dev_root).unwrap_or_default();
        }
    }

    if let Ok(local) = app.path().app_local_data_dir() {
        let root = local.join("sidecar");
        if root.join("app").is_dir() {
            if let Some(node) = bundled_node(app) {
                if let Some(sidecar) = from_root(&root, node) {
                    let is_newer = crate::update::compare_versions_public(
                        &sidecar.version,
                        &bundled_version,
                    ) == std::cmp::Ordering::Greater;
                    if is_newer {
                        crate::logging::line(
                            "sidecar",
                            &format!(
                                "使用热更新侧车 {}（pi-web {}）",
                                root.display(),
                                sidecar.version
                            ),
                        );
                        return Ok(sidecar);
                    }
                    crate::logging::line(
                        "sidecar",
                        &format!(
                            "忽略过期的热更新侧车（{} <= 包内 {}），继续用包内的",
                            sidecar.version, bundled_version
                        ),
                    );
                }
            }
        }
        tried.push(root.display().to_string());
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        let root = resource_dir.join("pi-web");
        let node = root.join("runtime").join(node_bin_name());
        if let Some(sidecar) = from_root(&root, node) {
            crate::logging::line(
                "sidecar",
                &format!("使用包内侧车 {}（pi-web {}）", root.display(), sidecar.version),
            );
            return Ok(sidecar);
        }
        tried.push(root.display().to_string());
    }

    if let Some(dev_root) = dev_sidecar_root() {
        let dev_node = dev_root.join("runtime").join(node_bin_name());
        if let Some(sidecar) = from_root(&dev_root, dev_node) {
            crate::logging::line(
                "sidecar",
                &format!("使用开发目录侧车 {}（pi-web {}）", dev_root.display(), sidecar.version),
            );
            return Ok(sidecar);
        }
        tried.push(dev_root.display().to_string());
    }

    Err(format!(
        "找不到可用的侧车目录（已试：{}）。开发环境先跑 `npm run sidecar`。",
        tried.join(" / ")
    ))
}

// ---------------------------------------------------------------- ports

pub fn is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 端口必须稳定：WKWebView 的 localStorage 按 origin（含端口）隔离，随机端口会让
/// 侧栏偏好/主题/语言/草稿每次重置。所以优先沿用 config 里的端口，再顺延 10 个。
pub fn pick_port(preferred: u16) -> u16 {
    for offset in 0..PORT_CANDIDATES {
        let Some(port) = preferred.checked_add(offset) else { break };
        if port != 0 && is_free(port) {
            return port;
        }
    }
    // 极端情况：让系统给一个（此时提醒用户偏好不会保留）
    if let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) {
        if let Ok(addr) = listener.local_addr() {
            return addr.port();
        }
    }
    preferred
}

// ---------------------------------------------------------------- spawn

pub struct Running {
    pub child: Child,
    pub port: u16,
    pub launcher_pid: u32,
}

/// 跑一次登录 shell，把交互式 rc 写进 PATH 的结果取回来。
///
/// 为什么必须这么干：从 Finder/Dock 启动的 GUI 进程只继承
/// `/usr/bin:/bin:/usr/sbin:/sbin`，而 nvm / pnpm / asdf / volta / deno 这些
/// 都是 `.zshrc` 里自己 append 的 —— 不取登录 shell 的 PATH，pi agent 在会话里
/// 就看不到 `mnemon`、`pnpm`、`node`(nvm 版) 这些命令。
///
/// 用 `-l`（读 .zprofile）+ `-i`（读 .zshrc）+ `-c`；stderr 丢掉（starship 之类
/// 会往 stderr 打 TERM=dumb 的抱怨），只认 stdout 里最后一行像 PATH 的内容。
/// 2s 超时，失败就当作没有，退回内置列表，绝不让壳启动被 rc 拖死。
fn login_path_cache_file() -> Option<PathBuf> {
    app_dirs::config_dir().map(|d| d.join("login-path.txt"))
}

/// 从 `zsh -lic` 拿登录 shell 的 PATH；失败/超时返回 None。
///
/// 实测：从 Finder 启动的 .app 里这一条会**卡住**（同一条命令在终端里 0.3–1.2s 就回），
/// 所以这里只等 1.2s；拿到就缓存到 app config 目录，下次启动直接用缓存，不再付这个时间。
fn probe_login_shell_path() -> Option<String> {
    if !cfg!(unix) {
        return None;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let started = Instant::now();
    let mut child = match Command::new(&shell)
        .arg("-lic")
        .arg("printf %s \"$PATH\"")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            crate::logging::line("path", &format!("无法启动登录 shell {shell}：{error}"));
            return None;
        }
    };

    let deadline = Instant::now() + Duration::from_millis(1200);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                crate::logging::line(
                    "path",
                    &format!(
                        "登录 shell 取 PATH 超过 1.2s（已等 {}ms），放弃；改用缓存/常见目录兜底",
                        started.elapsed().as_millis()
                    ),
                );
                return None;
            }
        }
    }

    let mut out = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let candidate = out
        .lines()
        .rev()
        .find(|line| line.contains('/') && line.contains(':'))?
        .trim()
        .to_string();
    if candidate.matches('/').count() < 2 {
        return None;
    }
    crate::logging::line(
        "path",
        &format!(
            "登录 shell PATH 取回 {} 项（{}ms）",
            candidate.split(':').count(),
            started.elapsed().as_millis()
        ),
    );
    Some(candidate)
}

static LOGIN_PATH_PROBE_STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// 后台跑一次登录 shell 探测，成功后写缓存给下次启动用。
/// 不在启动路径上等它：GUI 启动时这条命令实测会卡住（同一命令在终端里 0.3–1.2s 就回）。
fn trigger_login_path_probe() {
    if LOGIN_PATH_PROBE_STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(|| {
        if let Some(path) = probe_login_shell_path() {
            write_login_path_cache(&path);
            crate::logging::line("path", "登录 shell PATH 已缓存，下次启动生效");
        }
    });
}

fn read_login_path_cache() -> Option<String> {
    let path = login_path_cache_file()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let text = text.trim().to_string();
    if text.matches('/').count() < 2 {
        return None;
    }
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())?;
    if age > Duration::from_secs(7 * 24 * 3600) {
        return None;
    }
    Some(text)
}

fn write_login_path_cache(value: &str) {
    let Some(path) = login_path_cache_file() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, value);
}

/// 读不到登录 shell 时的兜底：把常见的版本管理器/用户级 bin 目录按优先级列出来。
/// 这一步是纯 readdir，零开销，保证即使 `zsh -lic` 卡住，nvm（mnemon/pnpm）、
/// homebrew、deno、go 这些也照样在 PATH 里。
fn fallback_user_bin_dirs() -> Vec<PathBuf> {
    let Some(home) = crate::discovery::home_dir() else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = Vec::new();

    // 1) 用户级 bin 排在最前，顺序对齐常见的登录 shell PATH：
    //    终端里 `node` 命中的是 ~/.local/bin/node（不是 nvm 那个），这里保持一致。
    for relative in [
        ".local/bin",
        ".mimocode/bin",
        ".codebuddy/bin",
        ".minimax/bin",
        ".kimi-code/bin",
        ".opencode/bin",
        ".mavis/bin",
        "bin",
        "flutter/bin",
        ".dotnet/tools",
        ".deno/bin",
        ".bun/bin",
        ".cargo/bin",
        "go/bin",
        "anaconda3/condabin",
        "miniconda3/condabin",
        ".orbstack/bin",
        ".x-cmd.root/bin",
        "Library/pnpm/bin",
        "Library/pnpm",
        ".volta/bin",
        ".asdf/shims",
        ".local/share/mise/shims",
        ".fnm/aliases/default/bin",
        ".nodenv/shims",
    ] {
        dirs.push(home.join(relative));
    }

    // 2) nvm：优先 alias/default 指向的那个"当前版本"，其余按版本号从新到旧。
    let active = std::fs::read_to_string(home.join(".nvm/alias/default"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let nvm_root = home.join(".nvm/versions/node");
    if let Ok(entries) = std::fs::read_dir(&nvm_root) {
        let mut versions: Vec<(Vec<u32>, String, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let numbers: Vec<u32> = name
                    .trim_start_matches('v')
                    .split('.')
                    .filter_map(|part| part.parse::<u32>().ok())
                    .collect();
                if numbers.is_empty() {
                    return None;
                }
                Some((numbers, name, entry.path().join("bin")))
            })
            .collect();
        let is_active = |name: &str| {
            active.as_deref().map_or(false, |alias| {
                name == format!("v{alias}") || name.starts_with(&format!("v{alias}.")) || name.contains(alias)
            })
        };
        versions.sort_by(|a, b| {
            is_active(&b.1)
                .cmp(&is_active(&a.1))
                .then_with(|| b.0.cmp(&a.0))
        });
        dirs.extend(versions.into_iter().map(|(_, _, path)| path));
    }

    // 3) 系统/包管理器目录
    for absolute in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/go/bin",
        "/usr/local/share/dotnet",
    ] {
        dirs.push(PathBuf::from(absolute));
    }
    // 最后的兜底：本机 node 二进制所在（~/.local/bin/node 是它的软链）
    dirs.push(home.join(".hermes/node/bin"));
    dirs
}

/// 最终给侧车的 PATH：登录 shell（或缓存）→ 常见用户目录兜底 → 继承值 → 包内 runtime（最后）。
///
/// 包内 runtime 放最后是刻意的：侧车自己用 `runtime/node` 的绝对路径启动，不依赖 PATH；
/// 而会话里执行的命令应该和终端一致（你终端里 `node` 是 ~/.local/bin 那个 v22，
/// `pnpm`/`mnemon` 是 nvm 里的），包内 Node 只做"用户完全没有 node"时的兜底。
fn path_with_runtime(sidecar: &Sidecar, config: &Config) -> String {
    let mut parts: Vec<PathBuf> = Vec::new();
    let mut source = "内置目录";

    if config.inherit_login_shell_path {
        if let Some(cached) = read_login_path_cache() {
            part_extend(&mut parts, &cached);
            source = "登录 shell 缓存";
        } else {
            // 首次启动（或缓存过期）：不等登录 shell，直接用常见目录 + 后台探测
            for dir in fallback_user_bin_dirs() {
                parts.push(dir);
            }
            source = "常见目录兜底（登录 shell 后台探测中）";
            trigger_login_path_probe();
        }
    }
    if parts.is_empty() {
        for dir in fallback_user_bin_dirs() {
            parts.push(dir);
        }
        source = "常见目录兜底";
    }

    if let Some(existing) = std::env::var_os("PATH") {
        parts.extend(std::env::split_paths(&existing));
    }
    if let Some(runtime_dir) = sidecar.node.parent() {
        parts.push(runtime_dir.to_path_buf());
    }

    let mut seen = std::collections::HashSet::new();
    parts.retain(|p| !p.as_os_str().is_empty() && seen.insert(p.clone()));
    let joined = std::env::join_paths(parts)
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let entries: Vec<&str> = joined.split(':').collect();
    let has_nvm = entries.iter().any(|e| e.contains(".nvm/"));
    let has_mnemon = entries.iter().any(|dir| Path::new(dir).join("mnemon").is_file());
    crate::logging::line(
        "path",
        &format!(
            "侧车 PATH {} 项（来源：{source}，nvm {}，mnemon {}）：{}",
            entries.len(),
            if has_nvm { "在" } else { "不在" },
            if has_mnemon { "可见" } else { "不可见" },
            entries.iter().take(6).copied().collect::<Vec<_>>().join(" : ")
        ),
    );
    joined
}

fn part_extend(parts: &mut Vec<PathBuf>, joined_path: &str) {
    parts.extend(std::env::split_paths(joined_path));
}

pub fn spawn(sidecar: &Sidecar, config: &Config, port: u16) -> Result<Running, String> {
    let mut command = Command::new(&sidecar.node);
    command
        .arg(&sidecar.entry)
        .arg("--no-open")
        .arg("-p")
        .arg(port.to_string())
        .arg("-H")
        .arg("127.0.0.1")
        .current_dir(&sidecar.pkg_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    command
        .env("PI_WEB_NO_OPEN", "1")
        .env("PI_WEB_HOSTNAME", "127.0.0.1")
        .env("PORT", port.to_string())
        .env("PI_WEB_SKIP_VERSION_CHECK", "1")
        // GUI 进程没有 locale，不设的话终端面板会乱码、部分命令也认不出 UTF-8
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .env("TERM", "xterm-256color")
        .env("PATH", path_with_runtime(sidecar, config));

    if let Some(proxy) = config.proxy.as_ref().filter(|p| !p.trim().is_empty()) {
        command
            .env("HTTP_PROXY", proxy)
            .env("HTTPS_PROXY", proxy)
            .env("http_proxy", proxy)
            .env("https_proxy", proxy)
            .env("NO_PROXY", "localhost,127.0.0.1,::1")
            .env("no_proxy", "localhost,127.0.0.1,::1");
    }
    if let Some(agent_dir) = config.pi_coding_agent_dir.as_ref().filter(|d| !d.trim().is_empty()) {
        command.env("PI_CODING_AGENT_DIR", agent_dir);
    }
    for (key, value) in &config.extra_env {
        command.env(key, value);
    }

    // 让 launcher 及其 next 子进程成为独立进程组，收尾时一次信号打整棵树
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    // 不给 Node 弹控制台窗口（Windows）
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    crate::logging::line(
        "sidecar",
        &format!(
            "启动 {} {} --no-open -p {port} -H 127.0.0.1（cwd {}）",
            sidecar.node.display(),
            sidecar.entry.display(),
            sidecar.pkg_dir.display()
        ),
    );

    let mut child = command.spawn().map_err(|e| format!("启动侧车失败：{e}"))?;
    let launcher_pid = child.id();

    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                crate::logging::line("pi-web", &line);
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                crate::logging::line("pi-web!", &line);
            }
        });
    }

    Ok(Running {
        child,
        port,
        launcher_pid,
    })
}

// ---------------------------------------------------------------- stop

fn signal_group(pid: u32, signal: &str) {
    #[cfg(unix)]
    {
        // 负 pid = 整个进程组（spawn 时 process_group(0)，组长就是这个 pid）
        let _ = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // 组长可能已经退出，顺手直接打一下 pid
        let _ = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = signal;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// 优雅停止：SIGTERM → 最多等 6s（launcher 内部 5s 兜底）→ 整组 SIGKILL。
pub fn stop(mut running: Running) {
    let pid = running.launcher_pid;
    crate::logging::line("sidecar", &format!("停止侧车（launcher pid {pid}）"));
    signal_group(pid, "TERM");

    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        match running.child.try_wait() {
            Ok(Some(status)) => {
                crate::logging::line("sidecar", &format!("侧车已退出（{status}）"));
                break;
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                crate::logging::line("sidecar", "侧车 6s 内没退出，强杀进程组");
                signal_group(pid, "KILL");
                let _ = running.child.wait();
                break;
            }
        }
    }
}

// ------------------------------------------------------- 自家孤儿进程清理

fn own_state_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("running.json"))
}

#[derive(Serialize, Deserialize)]
struct OwnState {
    pid: u32,
    port: u16,
}

pub fn write_own_state(app: &tauri::AppHandle, running: &Running) {
    let Some(path) = own_state_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let state = OwnState {
        pid: running.launcher_pid,
        port: running.port,
    };
    if let Ok(text) = serde_json::to_string(&state) {
        let _ = std::fs::write(path, text);
    }
}

pub fn clear_own_state(app: &tauri::AppHandle) {
    if let Some(path) = own_state_path(app) {
        let _ = std::fs::remove_file(path);
    }
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// pid 复用保护（首选）：pi-web 自己会为每个 launcher 写
/// `<agentDir>/pi-web-run/<pid>.json`，里面有 pid / port / version。
/// 三者都对得上，才认定这个 pid 就是我们的侧车。
fn matches_run_record(pid: u32, port: u16, sidecar: &Sidecar) -> bool {
    let Some(dir) = crate::discovery::run_record_dir() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(dir.join(format!("{pid}.json"))) else {
        return false;
    };
    let Ok(record) = serde_json::from_str::<crate::discovery::RunRecord>(&text) else {
        return false;
    };
    record.pid == Some(pid)
        && record.port == Some(port)
        && record.version.as_deref() == Some(sidecar.version.as_str())
}

/// pid 复用保护（退路）：命令行里含我们侧车入口路径才算。`ps` 在某些受限环境里
/// 不可用（返回错误），这时返回 false，让调用方走"不清理"的安全分支。
fn process_looks_like_ours(pid: u32, sidecar: &Sidecar) -> bool {
    #[cfg(unix)]
    {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.contains("pi-web.js") || text.contains(&*sidecar.entry.to_string_lossy())
            }
            _ => false,
        }
    }
    #[cfg(windows)]
    {
        let _ = (pid, sidecar);
        true
    }
}

/// 上次崩溃（SIGKILL / 断电）留下的服务：启动时先收掉，避免端口被占、也避免两份
/// 服务同时写同一份会话文件。
pub fn cleanup_previous(app: &tauri::AppHandle) {
    let Some(path) = own_state_path(app) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let _ = std::fs::remove_file(&path);
    let Ok(state) = serde_json::from_str::<OwnState>(&text) else { return };
    if !process_alive(state.pid) {
        crate::logging::line("sidecar", "上次运行的进程已不存在，无需清理");
        return;
    }
    if let Ok(sidecar) = resolve(app) {
        let ours = matches_run_record(state.pid, state.port, &sidecar)
            || process_looks_like_ours(state.pid, &sidecar);
        if !ours {
            crate::logging::line(
                "sidecar",
                &format!(
                    "pid {} 无法确认是我们的侧车（run record / 命令行都不匹配，可能是 pid 复用），跳过清理",
                    state.pid
                ),
            );
            return;
        }
    }
    crate::logging::line(
        "sidecar",
        &format!("清理上次遗留的侧车 pid {} 端口 {}", state.pid, state.port),
    );
    signal_group(state.pid, "TERM");
    std::thread::sleep(Duration::from_millis(800));
    if process_alive(state.pid) {
        signal_group(state.pid, "KILL");
    }
}
