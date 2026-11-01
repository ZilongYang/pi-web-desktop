//! 发现"已经在本机跑着的 pi-web 实例"（附着模式）。
//!
//! 契约来自 pi-web 的 `bin/run-records.js`：每个 launcher 在
//! `<agentDir>/pi-web-run/<launcherPid>.json` 写一条
//! `{pid, nextPid, port, hostname, url, version, startedAt}`，退出时删除。
//! 这里只读它，不写它 —— 写的是 pi-web 自己的职责。

use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub pid: Option<u32>,
    pub next_pid: Option<u32>,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub version: Option<String>,
    pub started_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExistingInstance {
    pub url: String,
    pub port: u16,
    pub version: String,
    pub record_path: PathBuf,
}

/// pi 的 agent 目录：`PI_CODING_AGENT_DIR`（支持 `~`）优先，否则 `~/.pi/agent`。
pub fn agent_dir() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("PI_CODING_AGENT_DIR") {
        let raw = configured.to_string_lossy().to_string();
        if !raw.is_empty() {
            if raw == "~" {
                return home_dir();
            }
            if let Some(rest) = raw.strip_prefix("~/") {
                return home_dir().map(|h| h.join(rest));
            }
            return Some(PathBuf::from(raw));
        }
    }
    home_dir().map(|h| h.join(".pi").join("agent"))
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

pub fn run_record_dir() -> Option<PathBuf> {
    agent_dir().map(|d| d.join("pi-web-run"))
}

/// 读出所有能解析的记录（坏文件直接跳过，不当成错误）。
pub fn read_records() -> Vec<(PathBuf, RunRecord)> {
    let Some(dir) = run_record_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(record) = serde_json::from_str::<RunRecord>(&text) {
                out.push((path, record));
            }
        }
    }
    out
}

/// run record 是"启动时写的"，进程可能早死了但文件还在（pi-web 只在正常退出时删）。
/// 端口又可能已经被别的实例接走，所以必须再确认 pid 还活着，否则会把陈记录当成活实例。
#[cfg(unix)]
fn record_process_alive(record: &RunRecord) -> bool {
    let alive = |pid: Option<u32>| {
        pid.filter(|p| *p > 0)
            .map(|pid| {
                std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    };
    alive(record.pid) || alive(record.next_pid)
}

#[cfg(not(unix))]
fn record_process_alive(record: &RunRecord) -> bool {
    let _ = record;
    true
}

fn probe_instance(record: &RunRecord) -> Option<ExistingInstance> {
    if !record_process_alive(record) {
        return None;
    }
    let port = record.port?;
    let url = record
        .url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
    if !url.starts_with("http://127.0.0.1:") && !url.starts_with("http://localhost:") {
        return None;
    }
    if !crate::probe::http_answers(port, Duration::from_millis(600)) {
        return None;
    }
    Some(ExistingInstance {
        url,
        port,
        version: record.version.clone().unwrap_or_default(),
        record_path: PathBuf::new(),
    })
}

/// 找「同版本且活着」的实例用于附着。
pub fn find_matching(version: &str) -> Option<ExistingInstance> {
    let mut candidates: Vec<ExistingInstance> = read_records()
        .into_iter()
        .filter_map(|(path, record)| {
            let mut instance = probe_instance(&record)?;
            instance.record_path = path;
            Some(instance)
        })
        .collect();
    // 端口小的优先（大概率是用户常驻的那个）
    candidates.sort_by_key(|c| c.port);
    candidates
        .into_iter()
        .find(|c| !version.is_empty() && c.version == version)
}

/// 找「活着但版本不一致」的实例（用来给用户提示）。
pub fn find_foreign(version: &str) -> Option<ExistingInstance> {
    let records = read_records();
    let mut found: Vec<ExistingInstance> = records
        .into_iter()
        .filter_map(|(path, record)| {
            let mut instance = probe_instance(&record)?;
            instance.record_path = path;
            Some(instance)
        })
        .collect();
    found.sort_by_key(|c| c.port);
    found.into_iter().find(|c| c.version != version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_record_shape() {
        let json = r#"{
            "pid": 22084,
            "nextPid": 22090,
            "port": 31234,
            "hostname": "127.0.0.1",
            "url": "http://127.0.0.1:31234",
            "version": "0.13.0",
            "startedAt": "2026-11-01T13:27:19.743Z"
        }"#;
        let record: RunRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.port, Some(31234));
        assert_eq!(record.next_pid, Some(22090));
        assert_eq!(record.version.as_deref(), Some("0.13.0"));
        assert_eq!(record.url.as_deref(), Some("http://127.0.0.1:31234"));
    }

    #[test]
    fn tolerates_partial_records() {
        let record: RunRecord = serde_json::from_str(r#"{"port": 30141}"#).unwrap();
        assert_eq!(record.port, Some(30141));
        assert!(record.url.is_none());
    }
}
