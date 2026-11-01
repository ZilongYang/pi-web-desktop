//! 极简文件日志：写 <app_log_dir>/sidecar.log，同时在内存里留最后 200 行给 splash 显示。
//! 刻意不引第三方日志库：桌面壳只需要"能定位问题"。

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const TAIL_LINES: usize = 200;
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

static STATE: OnceLock<LogState> = OnceLock::new();

struct LogState {
    file: Mutex<Option<File>>,
    path: Mutex<Option<PathBuf>>,
    tail: Mutex<VecDeque<String>>,
}

fn state() -> &'static LogState {
    STATE.get_or_init(|| LogState {
        file: Mutex::new(None),
        path: Mutex::new(None),
        tail: Mutex::new(VecDeque::new()),
    })
}

/// 初始化：创建日志目录，超过 5MB 就滚到 sidecar.log.1（只保留一份旧档）。
pub fn init(log_dir: PathBuf) -> std::io::Result<PathBuf> {
    fs::create_dir_all(&log_dir)?;
    let path = log_dir.join("sidecar.log");
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = fs::rename(&path, log_dir.join("sidecar.log.1"));
        }
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    *state().file.lock().unwrap() = Some(file);
    *state().path.lock().unwrap() = Some(path.clone());
    Ok(path)
}

/// 当前日志文件路径（菜单"打开日志目录"用）。
pub fn path() -> Option<PathBuf> {
    state().path.lock().ok().and_then(|p| p.clone())
}

/// 带时间戳写一行；时间戳用 UNIX ms，避免引 chrono。
pub fn line(scope: &str, message: &str) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let text = format!("{stamp} [{scope}] {message}");

    if let Ok(mut tail) = state().tail.lock() {
        tail.push_back(text.clone());
        while tail.len() > TAIL_LINES {
            tail.pop_front();
        }
    }
    if let Ok(mut file) = state().file.lock() {
        if let Some(f) = file.as_mut() {
            let _ = writeln!(f, "{text}");
            let _ = f.flush();
        }
    }
    eprintln!("{text}");
}

/// 最后 n 行（splash 失败页用）。
pub fn tail(n: usize) -> Vec<String> {
    state()
        .tail
        .lock()
        .map(|t| t.iter().rev().take(n).rev().cloned().collect())
        .unwrap_or_default()
}
