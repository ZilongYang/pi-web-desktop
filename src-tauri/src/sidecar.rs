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
/// 取消 L3（2026-11-04）后：把历史上热更新留在
/// `<app_local_data>/sidecar` 的 overlay 删掉，回到内置固定版本。
/// 在 resolve 之前调用；删不掉只记日志，不阻塞启动。
pub fn cleanup_overlay(app: &tauri::AppHandle) {
    let Ok(local) = app.path().app_local_data_dir() else {
        return;
    };
    let root = local.join("sidecar");
    if !root.exists() {
        return;
    }
    match std::fs::remove_dir_all(&root) {
        Ok(()) => crate::logging::line(
            "sidecar",
            &format!("已清理历史热更新侧车 {}（L3 已取消，回到内置版本）", root.display()),
        ),
        Err(error) => crate::logging::line(
            "sidecar",
            &format!("清理历史热更新侧车失败（忽略）：{error}"),
        ),
    }
}

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

fn login_path_cache_file() -> Option<PathBuf> {
    app_dirs::config_dir().map(|d| d.join("login-path.txt"))
}

/// 登录 shell PATH 缓存的有效期：超过就重新探测（探不到也继续用「缓存 + 兜底」并集）。
const LOGIN_PATH_CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// 缓存「够旧」就顺手后台刷新一次：不必等 7 天过期，换了 node 版本/装了新工具后最多一天自愈。
const LOGIN_PATH_CACHE_REFRESH_AFTER: Duration = Duration::from_secs(24 * 3600);
/// 登录 shell 探测的等待上限。探测永远在后台线程里跑，不占启动路径，所以给足时间
/// （旧版 1.2s 在构建/高负载时会被误判超时，而缓存一旦写不进去，下次启动还是老路径）。
const LOGIN_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// 同一个后台探测线程里最多试几次：一次失败不再等于「本进程终身放弃」。
const LOGIN_PROBE_ATTEMPTS: usize = 2;
/// 菜单「重新载入登录环境」用的宽松超时：用户主动触发，多等几秒没关系。
const LOGIN_PROBE_MANUAL_TIMEOUT: Duration = Duration::from_secs(8);

/// 从 `zsh -lic` 拿登录 shell 的 PATH；失败/超时返回 None。
///
/// 为什么必须这么干：从 Finder/Dock 启动的 GUI 进程只继承
/// `/usr/bin:/bin:/usr/sbin:/sbin`，而 nvm / pnpm / asdf / volta / deno 这些
/// 都是 `.zshrc` 里自己 append 的 —— 不取登录 shell 的 PATH，pi agent 在会话里
/// 就看不到 `mnemon`、`pnpm`、`node`(nvm 版) 这些命令。
///
/// 用 `-l`（读 .zprofile）+ `-i`（读 .zshrc）+ `-c`；stderr 丢掉（starship 之类
/// 会往 stderr 打 TERM=dumb 的抱怨），只认 stdout 里最后一行像 PATH 的内容。
fn probe_login_shell_path() -> Option<String> {
    probe_login_shell_path_with_timeout(LOGIN_PROBE_TIMEOUT)
}

fn probe_login_shell_path_with_timeout(timeout: Duration) -> Option<String> {
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

    // stdout 必须「边跑边读」：rc 往 stdout 打横幅时，管道写满（macOS 64 KB）会让
    // 子 shell 卡在 write 上永不退出 → try_wait 等不到 → 被误判成「超时」。
    //
    // 读到的东西先攒进共享 buffer，而不是等 read_to_string 读到 EOF 才交卷：rc 里起的
    // 后台进程如果继承了 stdout（nvm 懒加载 / antigen / conda hook 都常见），子 shell
    // 早已退出、PATH 也已经打印，但管道永远等不到 EOF —— 旧实现会把这份可用结果丢掉。
    // 这里 EOF 最多等 1s，等不到就用已经读到的内容。
    //
    // 按字节攒、最后一次性 lossy 解码：分块解码会在 4 KiB 边界切断多字节字符。
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let (sender, receiver) = std::sync::mpsc::channel::<bool>();
    match child.stdout.take() {
        Some(mut stdout) => {
            let output = output.clone();
            std::thread::spawn(move || {
                use std::io::Read;
                let mut chunk = [0u8; 4096];
                let mut read_ok = true;
                loop {
                    match stdout.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(read) => {
                            if let Ok(mut buffer) = output.lock() {
                                buffer.extend_from_slice(&chunk[..read]);
                            }
                        }
                        Err(_) => {
                            read_ok = false;
                            break;
                        }
                    }
                }
                let _ = sender.send(read_ok);
            });
        }
        None => {
            let _ = sender.send(false);
        }
    }

    let deadline = Instant::now() + timeout;
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
                        "登录 shell 取 PATH 超过 {}s（已等 {}ms），放弃；继续用缓存/兜底目录",
                        timeout.as_secs(),
                        started.elapsed().as_millis()
                    ),
                );
                return None;
            }
        }
    }

    let eof = receiver
        .recv_timeout(Duration::from_millis(1000))
        .unwrap_or(false);
    let bytes = output.lock().map(|buffer| buffer.clone()).unwrap_or_default();
    let out = String::from_utf8_lossy(&bytes).to_string();
    if !eof {
        crate::logging::line(
            "path",
            &format!(
                "登录 shell 已退出，但 stdout 1s 内没等到 EOF（可能有后台进程占着管道）；用已读到的 {} 字节继续",
                bytes.len()
            ),
        );
    } else if bytes.is_empty() {
        crate::logging::line("path", "登录 shell 已退出，stdout 为空");
    }
    let candidate = match out
        .lines()
        .rev()
        .find(|line| line.contains('/') && line.contains(':'))
    {
        Some(line) => line.trim().to_string(),
        None => {
            crate::logging::line(
                "path",
                &format!("登录 shell 输出里没有 PATH（共 {} 字节），放弃", out.len()),
            );
            return None;
        }
    };
    if candidate.matches('/').count() < 2 {
        crate::logging::line("path", "登录 shell 输出的 PATH 太短，放弃");
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

/// 上次发起探测的时间（UNIX ms）。0 = 还没探测过。
/// 用「最小间隔」而不是 `OnceLock`：长跑进程里侧车重启（菜单「重启服务」）也应该能重新
/// 探测一次，否则 24h 自愈在同一个 App 进程内只生效一次。
static LOGIN_PATH_PROBE_LAST_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
/// 两次后台探测的最小间隔：防止同一次启动里重复拉起 zsh。
const LOGIN_PROBE_MIN_INTERVAL: Duration = Duration::from_secs(600);

fn probe_allowed(last_probe_ms: u64, now_ms: u64) -> bool {
    last_probe_ms == 0
        || now_ms.saturating_sub(last_probe_ms) >= LOGIN_PROBE_MIN_INTERVAL.as_millis() as u64
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// 后台跑登录 shell 探测并刷新缓存。
///
/// 每 10 分钟最多拉起一个后台线程，但**线程内部最多试两次**：
/// 旧实现一次失败就终身放弃，而缓存又要等 7 天 TTL 才重新探测，等于永不自愈。
fn trigger_login_path_probe() {
    use std::sync::atomic::Ordering;
    let now = now_millis();
    let last = LOGIN_PATH_PROBE_LAST_MS.load(Ordering::SeqCst);
    if !probe_allowed(last, now) {
        crate::logging::line("path", "10 分钟内已探测过登录 shell PATH，本次跳过");
        return;
    }
    if LOGIN_PATH_PROBE_LAST_MS
        .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    std::thread::spawn(|| {
        for attempt in 1..=LOGIN_PROBE_ATTEMPTS {
            if let Some(path) = probe_login_shell_path() {
                write_login_path_cache(&path);
                return;
            }
            if attempt < LOGIN_PROBE_ATTEMPTS {
                crate::logging::line(
                    "path",
                    &format!("探测失败（第 {attempt} 次），500ms 后重试"),
                );
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        crate::logging::line(
            "path",
            "后台探测重试后仍未取到登录 shell PATH；继续用「缓存 + 兜底目录」并集（下次启动再试）",
        );
    });
}

struct LoginPathCache {
    value: String,
    age: Duration,
    /// 缓存里已经不存在的目录 —— nvm 换版本后就是这些，命中即视为「缓存陈旧」。
    missing_dirs: Vec<PathBuf>,
}

impl LoginPathCache {
    /// 是否需要后台重探测：有失效目录（换版本/删目录）或缓存超过一天。
    fn needs_refresh(&self) -> bool {
        !self.missing_dirs.is_empty() || self.age > LOGIN_PATH_CACHE_REFRESH_AFTER
    }

    fn describe(&self) -> String {
        let age = if self.age.as_secs() < 3600 {
            format!("{} 分钟前", self.age.as_secs() / 60)
        } else {
            format!("{:.1} 天前", self.age.as_secs_f64() / 86400.0)
        };
        if self.missing_dirs.is_empty() {
            format!("缓存 {age}")
        } else {
            format!("缓存 {age}，{} 条路径已失效", self.missing_dirs.len())
        }
    }
}

fn cache_is_fresh(age: Duration) -> bool {
    age <= LOGIN_PATH_CACHE_TTL
}

fn read_login_path_cache() -> Option<LoginPathCache> {
    read_login_path_cache_from(&login_path_cache_file()?)
}

/// 从指定路径读缓存（抽出来便于用临时文件做单测；文件读写逻辑这里才是 BUG 现场）。
fn read_login_path_cache_from(path: &Path) -> Option<LoginPathCache> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim().to_string();
    if text.matches('/').count() < 2 {
        return None;
    }
    let age = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())?;
    if !cache_is_fresh(age) {
        return None;
    }
    Some(LoginPathCache {
        missing_dirs: cached_missing_dirs(&text),
        value: text,
        age,
    })
}

/// 目录状态：区分「真的不存在」与「stat 失败」（权限 / TCC / 未挂载卷 / IO 错误）。
/// 后者不能当成「过期」或「垃圾条目」处理，否则会把实际存在的目录从缓存里误删。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CachedDirState {
    Present,
    Missing,
    Unknown,
}

fn cached_dir_state(path: &Path) -> CachedDirState {
    match std::fs::metadata(path) {
        Ok(_) => CachedDirState::Present,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CachedDirState::Missing,
        Err(_) => CachedDirState::Unknown,
    }
}

/// 缓存里已经不存在的目录（纯判断，便于单测）。
///
/// 只对**绝对路径**判失效：登录 shell 的 PATH 里可能有字面 `~/.dotnet/tools` 这种
/// 未展开条目（本机 .zshrc 就这么写过一条）。它不是「缓存过期」的证据 —— 否则探测
/// 成功写回缓存后，每次启动都会判「1 条失效」→ 反复探测，24h/7 天阈值都形同虚设。
/// stat 失败（权限/未挂载）也不算失效，见 `CachedDirState`。
fn cached_missing_dirs(cached: &str) -> Vec<PathBuf> {
    std::env::split_paths(cached)
        .filter(|dir| {
            let path = Path::new(dir.as_os_str());
            path.is_absolute() && cached_dir_state(path) == CachedDirState::Missing
        })
        .collect()
}

/// 缓存写入前的清洗：丢掉不存在/空/重复条目，只留捕获时真实存在的目录。
///
/// 为什么必须清洗：登录 shell 的 PATH 里本来就有 `~/.dotnet/tools`（字面波浪号）、
/// cryptexd 的 `/var/run/...` 伪目录、`/pkg/env/global/bin` 这类**永远不存在**的条目。
/// 原样写进缓存后，每次启动都会把「2 条路径已失效」判成缓存陈旧 → 反复拉起 zsh 探测，
/// 24h/7 天阈值全部失效。清洗后「失效」只会来自捕获时存在、之后消失的目录（正是
/// nvm 换版本的场景），自愈判断才准。不存在的 PATH 条目对命令查找本来就无用。
///
/// 返回 `None` 表示「清洗后一条不剩」（多半捕获到的不是 PATH 而是横幅），调用方应保留旧缓存。
fn normalize_login_path(value: &str) -> Option<String> {
    let mut dropped = 0usize;
    let mut kept_dirs = 0usize;
    let mut seen = std::collections::HashSet::new();
    let kept: Vec<PathBuf> = std::env::split_paths(value)
        .filter(|dir| {
            // stat 失败的条目保留（可能只是权限/挂载问题，不代表它消失了）
            let present = cached_dir_state(Path::new(dir.as_os_str())) != CachedDirState::Missing;
            let keep = !dir.as_os_str().is_empty() && present && seen.insert(dir.clone());
            if keep {
                kept_dirs += 1;
            } else {
                dropped += 1;
            }
            keep
        })
        .collect();
    if dropped > 0 {
        crate::logging::line(
            "path",
            &format!("缓存清洗：丢弃 {dropped} 条不存在/重复条目（保留 {kept_dirs} 条）"),
        );
    }
    if kept.is_empty() {
        crate::logging::line("path", "缓存清洗后为空（捕获内容不像 PATH），保留原缓存不覆盖");
        return None;
    }
    match std::env::join_paths(&kept) {
        Ok(joined) => Some(joined.to_string_lossy().to_string()),
        // 极端情况（条目里带 ':'、含非 UTF-8）：保留原值，别把整份缓存写坏
        Err(error) => {
            crate::logging::line(
                "path",
                &format!("缓存清洗后无法重新拼接 PATH（{error}），原样写入"),
            );
            Some(value.to_string())
        }
    }
}

fn write_login_path_cache(value: &str) {
    let Some(cleaned) = normalize_login_path(value) else {
        return;
    };
    let value = cleaned.as_str();
    let Some(path) = login_path_cache_file() else {
        crate::logging::line("path", "缓存写入失败：拿不到配置目录");
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            crate::logging::line(
                "path",
                &format!("缓存写入失败：建目录 {}：{error}", parent.display()),
            );
            return;
        }
    }
    let entries = std::env::split_paths(value)
        .filter(|entry| !entry.as_os_str().is_empty())
        .count();
    match std::fs::write(&path, value) {
        Ok(()) => {
            // 写后回读：把「写成功」和「真落地」分开，免得再出现「日志说已缓存、文件没变」的误判
            let readback = std::fs::read_to_string(&path).ok();
            if readback.as_deref() != Some(value) {
                crate::logging::line(
                    "path",
                    &format!(
                        "缓存写入后回读不一致：{}（{entries} 条 / {} 字节）",
                        path.display(),
                        value.len()
                    ),
                );
            } else {
                crate::logging::line(
                    "path",
                    &format!(
                        "登录 shell PATH 已缓存：{}（{entries} 条 / {} 字节），下次启动生效",
                        path.display(),
                        value.len()
                    ),
                );
            }
        }
        Err(error) => crate::logging::line(
            "path",
            &format!("缓存写入失败：{}：{error}", path.display()),
        ),
    }
}

/// 菜单「重新载入登录环境」正在进行的标记：连点两下不叠加探测/重启。
static LOGIN_PATH_RELOAD_IN_FLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 菜单「重新载入登录环境」：重新探测 → 成功才覆盖缓存 → 重启侧车。
/// 全程在后台线程，不阻塞 UI；失败保留原缓存并弹窗提示（headless 不弹）。
pub fn reload_login_environment(app: &tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    if LOGIN_PATH_RELOAD_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        crate::logging::line("path", "「重新载入登录环境」已在执行，忽略重复触发");
        return;
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        // panic 也要复位「进行中」标志，否则菜单会永久失效
        let _reset = ReloadInFlightGuard;
        crate::logging::line(
            "path",
            "「重新载入登录环境」：重新探测登录 shell（成功才覆盖缓存）",
        );
        match probe_login_shell_path_with_timeout(LOGIN_PROBE_MANUAL_TIMEOUT) {
            Some(path) => {
                write_login_path_cache(&path);
                if let Err(error) = crate::restart_sidecar_now(&handle) {
                    crate::logging::line("path", &format!("重新载入后重启侧车失败：{error}"));
                }
            }
            None => {
                crate::logging::line(
                    "path",
                    "「重新载入登录环境」失败：登录 shell 没有返回 PATH；保留原缓存",
                );
                notify_reload_failed(&handle);
            }
        }
    });
}

/// 用 Drop 保证「重新载入」的进行中标志一定会复位（线程 panic 也不会把菜单卡死）。
struct ReloadInFlightGuard;

impl Drop for ReloadInFlightGuard {
    fn drop(&mut self) {
        LOGIN_PATH_RELOAD_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

fn notify_reload_failed(app: &tauri::AppHandle) {
    if crate::update::headless() {
        return;
    }
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let _ = app
        .dialog()
        .message(
            "没能从登录 shell 取回 PATH（超时或输出异常）。\n\n\
             侧车会继续使用「缓存 + 兜底目录」拼出的 PATH，不会因此启动失败；\
             具体原因见日志（菜单「打开日志目录」→ sidecar.log 里的 [path] 行）。",
        )
        .title("Pi Web Desktop · 重新载入登录环境失败")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "知道了".to_string(),
            "关闭".to_string(),
        ))
        .blocking_show();
}

/// 读不到登录 shell 时的兜底：把常见的版本管理器/用户级 bin 目录按优先级列出来。
/// 这一步是纯 readdir，零开销。现在它不只是「兜底」——`path_with_runtime()` 永远把这份
/// 列表**并进** PATH（缓存永不自证完整），保证 nvm（mnemon/pnpm）、homebrew、deno、go
/// 这些即使缓存陈旧也照样可见。
fn fallback_user_bin_dirs() -> Vec<PathBuf> {
    crate::discovery::home_dir()
        .map(|home| fallback_user_bin_dirs_in(&home))
        .unwrap_or_default()
}

fn fallback_user_bin_dirs_in(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    // 1) 用户级 bin 排在最前，顺序对齐常见的登录 shell PATH（~/.local/bin 通常最靠前）
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

    // 2) nvm：优先 alias/default 指向的那个「当前版本」，其余按版本号从新到旧。
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
    dirs
}

/// 拼 PATH 条目的纯逻辑：缓存（可为空）→ 兜底目录 → 继承的 PATH → 包内 runtime；
/// 去空串、去重保序。抽成纯函数是为了能不改进程环境变量做单测。
fn compose_path_parts(
    cached: Option<&str>,
    fallback: &[PathBuf],
    inherited: Option<&std::ffi::OsStr>,
    runtime_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(cached) = cached {
        part_extend(&mut parts, cached);
    }
    parts.extend(fallback.iter().cloned());
    if let Some(inherited) = inherited {
        parts.extend(std::env::split_paths(inherited));
    }
    if let Some(runtime_dir) = runtime_dir {
        parts.push(runtime_dir.to_path_buf());
    }
    let mut seen = std::collections::HashSet::new();
    parts
        .into_iter()
        .filter(|part| !part.as_os_str().is_empty() && seen.insert(part.clone()))
        .collect()
}

/// 按 PATH 语义查找一个可执行文件，区分「可见 / 存在但不可执行 / 不可见」。
///
/// 旧实现只做 `dir/mnemon` 的 `is_file()`：目录已删除时仍会报「可见」，
/// 而且不校验可执行位 —— 排障时被它误导过（见 BUG-2026-11-05 疑点 2）。
enum ExecutableLookup {
    Found(PathBuf),
    NotExecutable(PathBuf),
    Missing,
}

impl ExecutableLookup {
    fn describe(&self) -> String {
        match self {
            Self::Found(path) => format!("可见 {}", path.display()),
            Self::NotExecutable(path) => format!("存在但不可执行 {}", path.display()),
            Self::Missing => "不可见".to_string(),
        }
    }
}

fn lookup_executable(entries: &[String], name: &str) -> ExecutableLookup {
    let mut not_executable: Option<PathBuf> = None;
    for dir in entries {
        let candidate = Path::new(dir).join(name);
        let Ok(metadata) = candidate.metadata() else { continue };
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                not_executable.get_or_insert(candidate);
                continue;
            }
        }
        return ExecutableLookup::Found(candidate);
    }
    match not_executable {
        Some(path) => ExecutableLookup::NotExecutable(path),
        None => ExecutableLookup::Missing,
    }
}

/// 从 PATH 条目里认出当前生效的 nvm 版本（只用于日志；只认真正存在的目录）。
fn active_nvm_version(entries: &[String]) -> Option<String> {
    entries.iter().find_map(|entry| {
        if !entry.contains("/.nvm/versions/node/") || !Path::new(entry).is_dir() {
            return None;
        }
        let version = entry.split("/.nvm/versions/node/").nth(1)?.split('/').next()?;
        (!version.is_empty()).then(|| version.to_string())
    })
}

/// 最终给侧车的 PATH：登录 shell 缓存 → 常见用户目录兜底 → 继承值 → 包内 runtime（最后）。
///
/// 两条硬规则（BUG-2026-11-05 的根因修复）：
/// 1. **缓存与兜底永远取并集**：缓存只代表「上次捕获时的登录 shell」，nvm 换版本后里面的
///    版本目录会失效；兜底目录是实时 readdir 出来的，必须始终并进去（缓存优先、去重保序）。
/// 2. **缓存陈旧就后台重探测**：缓存里有已不存在的目录、或缓存超过一天，就顺手刷新一次，
///    不等 7 天 TTL —— 旧实现缓存命中时根本不跑探测，「下次启动生效」永远是空话。
///
/// 包内 runtime 放最后是刻意的：侧车自己用 `runtime/node` 的绝对路径启动，不依赖 PATH；
/// 而会话里执行的命令应该和终端一致，包内 Node 只做「用户完全没有 node」时的兜底。
struct PathPlan {
    cached_value: Option<String>,
    source: &'static str,
    cache_note: Option<String>,
    probe: bool,
}

/// 决定「用不用缓存、来源怎么写、要不要后台探测」——纯函数，单测覆盖真正的分支
/// （BUG 就出在这里：旧实现缓存命中时既不并兜底、也不探测）。
fn plan_path_sources(inherit_login_shell_path: bool, cache: Option<LoginPathCache>) -> PathPlan {
    if !inherit_login_shell_path {
        // 明确关掉继承：不读缓存、不探测，只用兜底目录 + 继承值（保持旧行为）
        return PathPlan {
            cached_value: None,
            source: "常见目录兜底",
            cache_note: None,
            probe: false,
        };
    }
    match cache {
        Some(cache) => PathPlan {
            source: "登录 shell 缓存 + 兜底目录",
            probe: cache.needs_refresh(),
            cache_note: Some(cache.describe()),
            cached_value: Some(cache.value),
        },
        None => PathPlan {
            cached_value: None,
            source: "常见目录兜底（登录 shell 后台探测中）",
            cache_note: None,
            probe: true,
        },
    }
}

fn path_with_runtime(sidecar: &Sidecar, config: &Config) -> String {
    let fallback = fallback_user_bin_dirs();
    // 用户明确关掉继承时不读缓存（省一次文件读取 + stat）
    let cache = if config.inherit_login_shell_path {
        read_login_path_cache()
    } else {
        None
    };
    let plan = plan_path_sources(config.inherit_login_shell_path, cache);
    if plan.probe {
        trigger_login_path_probe();
    }

    let inherited = std::env::var_os("PATH");
    let runtime_dir = sidecar.node.parent();
    let parts = compose_path_parts(
        plan.cached_value.as_deref(),
        &fallback,
        inherited.as_deref(),
        runtime_dir,
    );
    let entries: Vec<String> = parts
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    let joined = std::env::join_paths(&parts)
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut extras: Vec<String> = Vec::new();
    if let Some(note) = plan.cache_note {
        extras.push(note);
    }
    if plan.probe {
        extras.push("已后台重新探测".to_string());
    }
    extras.push(format!(
        "nvm {}",
        match active_nvm_version(&entries) {
            Some(version) => format!("在 {version}"),
            None => "不在".to_string(),
        }
    ));
    extras.push(format!(
        "mnemon {}",
        lookup_executable(&entries, "mnemon").describe()
    ));
    crate::logging::line(
        "path",
        &format!(
            "侧车 PATH {} 项（来源：{}，{}）：{}",
            entries.len(),
            plan.source,
            extras.join("，"),
            entries.iter().take(6).map(String::as_str).collect::<Vec<_>>().join(" : ")
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

#[cfg(test)]
mod path_tests {
    use super::*;

    /// 每个测试独立的临时目录；不改进程的 HOME/PATH 等全局状态，测试可并行。
    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "piweb-sidecar-path-test-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn cache_and_fallback_are_unioned_with_cache_first() {
        let home = temp_home("union");
        let dead = home.join(".nvm/versions/node/v24.99.0/bin");
        let alive = home.join(".nvm/versions/node/v24.22.2/bin");
        std::fs::create_dir_all(&alive).unwrap();
        let cached = format!("{}:/usr/bin:/bin", dead.display());
        let fallback = vec![
            alive.clone(),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/opt/homebrew/bin"),
        ];

        let parts = compose_path_parts(
            Some(&cached),
            &fallback,
            Some(std::ffi::OsStr::new("/usr/bin:/usr/sbin")),
            Some(Path::new("/rt")),
        );

        assert_eq!(parts.first(), Some(&dead), "缓存条目保持最前（即使已失效）");
        assert!(parts.contains(&alive), "兜底枚举的 nvm bin 必须并集进来");
        assert!(parts.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert_eq!(
            parts.iter().filter(|p| *p == Path::new("/usr/bin")).count(),
            1,
            "跨来源重复条目要合并"
        );
        assert_eq!(parts.last(), Some(&PathBuf::from("/rt")), "包内 runtime 最后");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn empty_and_duplicate_entries_are_dropped() {
        let parts = compose_path_parts(Some(":/a::/a:/b:"), &[], None, None);
        assert_eq!(parts, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn missing_cached_dirs_are_detected() {
        let home = temp_home("missing");
        let alive = home.join("alive");
        std::fs::create_dir_all(&alive).unwrap();
        let dead = home.join("dead/bin");
        // 字面 "~/.dotnet/tools"（本机真实登录 PATH 里就有）不是「缓存过期」的证据
        let cached = format!("{}:{}:~/.dotnet/tools", dead.display(), alive.display());
        assert_eq!(cached_missing_dirs(&cached), vec![dead]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cache_plan_unions_fallback_and_probes_when_stale() {
        let fresh = LoginPathCache {
            value: "/cached".into(),
            age: Duration::from_secs(600),
            missing_dirs: Vec::new(),
        };
        let plan = plan_path_sources(true, Some(fresh));
        assert_eq!(plan.source, "登录 shell 缓存 + 兜底目录", "缓存命中也要标明并集来源");
        assert_eq!(plan.cached_value.as_deref(), Some("/cached"));
        assert!(!plan.probe, "刚写过且无失效目录：不该重复探测");

        let stale = LoginPathCache {
            value: "/cached".into(),
            age: Duration::from_secs(600),
            missing_dirs: vec![PathBuf::from("/dead")],
        };
        let plan = plan_path_sources(true, Some(stale));
        assert!(plan.probe, "缓存里有失效目录（nvm 换版本）必须后台重探测");
        assert_eq!(plan.cached_value.as_deref(), Some("/cached"), "陈旧缓存仍保留在原位（并集）");

        let plan = plan_path_sources(true, None);
        assert!(plan.probe, "缓存失效（过期/首次启动）要探测");
        assert_eq!(plan.source, "常见目录兜底（登录 shell 后台探测中）");

        let plan = plan_path_sources(false, None);
        assert!(!plan.probe, "用户明确关掉继承时不探测");
        assert_eq!(plan.source, "常见目录兜底");
    }

    #[test]
    fn reads_cache_file_and_reports_missing_dirs() {
        let home = temp_home("cachefile");
        let dead = home.join("dead/bin");
        let cache_path = home.join("login-path.txt");
        std::fs::write(
            &cache_path,
            format!("{}:/usr/bin:~/literal", dead.display()),
        )
        .unwrap();
        let cache = read_login_path_cache_from(&cache_path).expect("应读到缓存");
        assert_eq!(cache.missing_dirs, vec![dead]);
        assert!(cache.value.contains("/usr/bin"));

        std::fs::write(&cache_path, "too-short").unwrap();
        assert!(read_login_path_cache_from(&cache_path).is_none(), "不像 PATH 的内容不算缓存");
        assert!(read_login_path_cache_from(&home.join("no-such-file")).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cache_write_cleans_missing_entries_so_staleness_is_meaningful() {
        let home = temp_home("normalize");
        let alive = home.join("alive");
        std::fs::create_dir_all(&alive).unwrap();
        let raw = format!(
            "{}:/usr/bin:~/literal:{}:{}/",
            home.join("gone").display(),
            alive.display(),
            alive.display()
        );
        let cleaned = normalize_login_path(&raw).expect("清洗后仍有内容");
        assert!(!cleaned.contains("~/literal"), "字面 ~ 条目要丢掉（它永远不存在）");
        assert!(!cleaned.contains("gone"), "不存在的目录不写进缓存");
        assert_eq!(
            cleaned.matches(&*alive.display().to_string()).count(),
            1,
            "重复条目去重"
        );

        let path = home.join("login-path.txt");
        std::fs::write(&path, &cleaned).unwrap();
        let cache = read_login_path_cache_from(&path).expect("清洗后的缓存可读");
        assert!(cache.missing_dirs.is_empty(), "清洗后不应再有失效目录");
        assert!(!cache.needs_refresh(), "清洗后的缓存不该反复触发探测");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn newly_missing_cached_dir_marks_cache_stale() {
        let home = temp_home("nvm-switch");
        let bin = home.join(".nvm/versions/node/v24.22.2/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let path = home.join("login-path.txt");
        std::fs::write(&path, bin.display().to_string()).unwrap();
        assert!(!read_login_path_cache_from(&path)
            .expect("应读到缓存")
            .needs_refresh());

        // 模拟 nvm 换版本：缓存里这条目录消失 → 下一次读必须判「陈旧」触发重探测
        std::fs::remove_dir_all(&bin).unwrap();
        let cache = read_login_path_cache_from(&path).expect("缓存本身未过期");
        assert_eq!(cache.missing_dirs, vec![bin]);
        assert!(cache.needs_refresh(), "版本目录消失是换版本的信号，必须重探测");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn dir_state_distinguishes_missing_from_present() {
        let home = temp_home("dirstate");
        let alive = home.join("alive");
        std::fs::create_dir_all(&alive).unwrap();
        assert_eq!(cached_dir_state(&alive), CachedDirState::Present);
        assert_eq!(cached_dir_state(&home.join("gone")), CachedDirState::Missing);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn normalize_refuses_to_overwrite_cache_with_garbage() {
        let home = temp_home("garbage");
        let raw = format!(
            "{}/bin:{}/global/bin",
            home.join("gone1").display(),
            home.join("gone2").display()
        );
        // 捕获到的是横幅之类的东西：清洗后一条不剩 → 返回 None，调用方保留旧缓存
        assert!(normalize_login_path(&raw).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn probe_is_throttled_but_allows_sidecar_restart_later() {
        assert!(probe_allowed(0, 1_000), "首次启动必须允许探测");
        assert!(!probe_allowed(10_000, 10_000 + 60_000), "10 分钟内不重复拉起 zsh");
        assert!(probe_allowed(10_000, 10_000 + 600_000), "超过最小间隔允许再探测");
        // 时钟回拨（now < last）不应 panic；按「刚探测过」保守跳过本次，不会永久锁死
        assert!(!probe_allowed(10_000, 5_000));
    }

    #[test]
    fn cache_ttl_and_refresh_rules() {
        // 7 天 TTL 边界（超过才算过期，走兜底 + 探测分支）
        assert!(cache_is_fresh(Duration::from_secs(7 * 24 * 3600)));
        assert!(!cache_is_fresh(Duration::from_secs(7 * 24 * 3600 + 1)));

        // 命中但过旧 / 含失效目录 → 需要后台重探测，否则「下次启动生效」永远不生效
        let fresh = LoginPathCache {
            value: "/a".into(),
            age: Duration::from_secs(3600),
            missing_dirs: Vec::new(),
        };
        assert!(!fresh.needs_refresh());
        let old = LoginPathCache {
            value: "/a".into(),
            age: Duration::from_secs(25 * 3600),
            missing_dirs: Vec::new(),
        };
        assert!(old.needs_refresh(), "超过一天要顺手刷新");
        let stale = LoginPathCache {
            value: "/a".into(),
            age: Duration::from_secs(60),
            missing_dirs: vec![PathBuf::from("/dead")],
        };
        assert!(stale.needs_refresh(), "缓存里有失效目录就必须重探测");
    }

    #[test]
    fn fallback_enumerates_nvm_and_has_no_dead_hermes_dir() {
        let home = temp_home("fallback");
        let active = home.join(".nvm/versions/node/v24.22.2/bin");
        let newer = home.join(".nvm/versions/node/v26.8.2/bin");
        std::fs::create_dir_all(&active).unwrap();
        std::fs::create_dir_all(&newer).unwrap();
        std::fs::create_dir_all(home.join(".nvm/alias")).unwrap();
        std::fs::write(home.join(".nvm/alias/default"), "24\n").unwrap();

        let dirs = fallback_user_bin_dirs_in(&home);
        assert!(dirs.contains(&active), "alias/default 指向的版本必须在兜底里");
        assert!(dirs.contains(&newer), "其它已装版本也要兜底");
        assert!(
            dirs.iter().position(|d| *d == active).unwrap()
                < dirs.iter().position(|d| *d == newer).unwrap(),
            "alias/default 指向的版本排在前面"
        );
        assert!(
            !dirs.iter().any(|d| d.to_string_lossy().contains(".hermes")),
            "已删除的 ~/.hermes/node/bin 不应再当兜底目录"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn executable_lookup_distinguishes_visible_from_missing() {
        let dir = temp_home("exec");
        let bin = dir.join("mnemon");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let entries = vec![dir.to_string_lossy().to_string()];
        match lookup_executable(&entries, "mnemon") {
            ExecutableLookup::Found(path) => assert_eq!(path, bin),
            other => panic!("应为可见，实际：{}", other.describe()),
        }

        // 目录不存在时旧实现靠 is_file() 也会走到「不可见」；这里锁定「条目指向不存在的目录」
        let entries = vec![dir.join("no-such-dir").to_string_lossy().to_string()];
        assert!(matches!(
            lookup_executable(&entries, "mnemon"),
            ExecutableLookup::Missing
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();
            match lookup_executable(&[dir.to_string_lossy().to_string()], "mnemon") {
                ExecutableLookup::Found(_) => panic!("没有可执行位不应报可见"),
                _ => {}
            }
            let entries = vec![dir.to_string_lossy().to_string()];
            match lookup_executable(&entries, "mnemon") {
                ExecutableLookup::NotExecutable(path) => assert_eq!(path, bin),
                other => panic!("应为「存在但不可执行」，实际：{}", other.describe()),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn active_nvm_version_ignores_deleted_version_dir() {
        let home = temp_home("nvmver");
        let alive = home.join(".nvm/versions/node/v24.22.2/bin");
        std::fs::create_dir_all(&alive).unwrap();
        let entries = vec![
            home.join(".nvm/versions/node/v24.14.0/bin")
                .to_string_lossy()
                .to_string(),
            alive.to_string_lossy().to_string(),
        ];
        assert_eq!(active_nvm_version(&entries).as_deref(), Some("v24.22.2"));
        assert_eq!(
            active_nvm_version(&[home.join(".nvm/versions/node/v24.14.0/bin")
                .to_string_lossy()
                .to_string()]),
            None,
            "只剩失效版本目录时不能报「nvm 在」"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
