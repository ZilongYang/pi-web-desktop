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
        }
    }
}

fn config_path(app: &tauri::AppHandle) -> Option<PathBuf> {
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

fn validate(dir: &Path) -> Option<Sidecar> {
    let node = dir.join("runtime").join(node_bin_name());
    let pkg_dir = dir
        .join("app")
        .join("node_modules")
        .join("@agegr")
        .join("pi-web");
    let entry = pkg_dir.join("bin").join("pi-web.js");
    if !node.is_file() || !entry.is_file() {
        return None;
    }
    let version = read_sidecar_version(dir)
        .or_else(|| {
            let text = std::fs::read_to_string(pkg_dir.join("package.json")).ok()?;
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            json.get("version")?.as_str().map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    Some(Sidecar {
        dir: dir.to_path_buf(),
        node,
        pkg_dir,
        entry,
        version,
    })
}

/// manifest.json 由 scripts/fetch-sidecar.mjs 生成。
pub fn read_sidecar_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("piWebVersion")?.as_str().map(|s| s.to_string())
}

/// bundle 里在 Resources/pi-web；dev 下在仓库根的 sidecar/（不复制进 target）。
pub fn resolve(app: &tauri::AppHandle) -> Result<Sidecar, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("pi-web"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("sidecar"));

    let mut tried = Vec::new();
    for candidate in candidates {
        let shown = candidate.display().to_string();
        if let Some(sidecar) = validate(&candidate) {
            crate::logging::line("sidecar", &format!("使用侧车目录 {shown}（pi-web {}）", sidecar.version));
            return Ok(sidecar);
        }
        tried.push(shown);
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

fn path_with_runtime(sidecar: &Sidecar) -> String {
    let mut parts: Vec<PathBuf> = vec![sidecar
        .node
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()];
    if let Some(existing) = std::env::var_os("PATH") {
        parts.extend(std::env::split_paths(&existing));
    }
    // GUI 启动的 app 只有 /usr/bin:/bin:/usr/sbin:/sbin，补上包管理器与用户 bin
    if let Some(home) = crate::discovery::home_dir() {
        parts.push(home.join(".local/bin"));
        parts.push(home.join(".bun/bin"));
        parts.push(home.join(".cargo/bin"));
        parts.push(home.join(".hermes/node/bin"));
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
        parts.push(PathBuf::from(dir));
    }
    let mut seen = std::collections::HashSet::new();
    parts.retain(|p| !p.as_os_str().is_empty() && seen.insert(p.clone()));
    std::env::join_paths(parts)
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
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
        .env("PATH", path_with_runtime(sidecar));

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
