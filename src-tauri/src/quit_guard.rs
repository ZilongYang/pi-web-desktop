//! 退出保护（F22）：**任何退出都要确认**（⌘Q / 菜单退出 / 托盘退出）。
//!
//! 用户 2026-11-04 定的规则：不区分情况，退出前一律弹「确定要退出吗？」；
//! 如果 pi-web 里有会话正在跑，再额外说明「退出会中断 N 个会话」
//! （agent 跑在侧车里，退出会停掉侧车，未完成的回复/工具调用会丢失）。
//!
//! 流程：第一次 ExitRequested 一律 `prevent_exit()` 拦下 → 后台线程查一次
//! `GET /api/sessions` 的 `runningSessionIds` → 弹确认框：
//!   · 确定：放行（再次 exit），随后正常停侧车退出；
//!   · 取消：什么都不做，会话继续跑。
//! 用 `CONFIRMED` 标记避免二次弹窗（第二次 exit 会再次走到 ExitRequested）。
//! 关窗按钮不算退出（F16 会隐藏到菜单栏），不经过这里。
//!
//! 注意：更新/重启（`RESTART_EXIT_CODE`）必须直接放行，否则会把更新流程卡住。

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

static CONFIRMED: AtomicBool = AtomicBool::new(false);

/// 处理一次 `RunEvent::ExitRequested`。
///
/// 返回 `true` = 这次退出可以继续（调用方负责停侧车）；`false` = 已拦下，稍后异步决定。
pub fn on_exit_requested(app: &AppHandle, api: &tauri::ExitRequestApi, code: Option<i32>) -> bool {
    // 更新后自动重启（L2/L3）不要弹确认框
    if code == Some(tauri::RESTART_EXIT_CODE) || CONFIRMED.load(Ordering::SeqCst) {
        return true;
    }

    api.prevent_exit();
    let handle = app.clone();
    std::thread::spawn(move || {
        let running = running_sessions(&handle);
        let message = if running.is_empty() {
            "确定要退出 Pi Web Desktop 吗？\n\n退出会一并停止 pi-web 服务（可由菜单栏图标重新打开）。"
                .to_string()
        } else {
            format!(
                "有 {} 个会话正在运行。\n\n现在退出会中断它们（未完成的回复与工具调用会丢失）。\n\n仍要退出吗？",
                running.len()
            )
        };

        let confirmed = if headless() {
            crate::logging::line("quit", "headless：自动确认退出");
            true
        } else {
            handle
                .dialog()
                .message(message)
                .title("Pi Web Desktop · 退出确认")
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "退出".to_string(),
                    "取消".to_string(),
                ))
                .blocking_show()
        };

        if confirmed {
            crate::logging::line(
                "quit",
                &format!("确认退出（运行中的会话 {} 个）", running.len()),
            );
            CONFIRMED.store(true, Ordering::SeqCst);
            handle.exit(0);
        } else {
            crate::logging::line("quit", "取消退出，会话继续运行");
        }
    });
    false
}

/// 退出流程是否已被确认过（单测/日志用）。
pub fn is_confirmed() -> bool {
    CONFIRMED.load(Ordering::SeqCst)
}

fn headless() -> bool {
    std::env::var("PIWEB_UPDATE_HEADLESS").ok().as_deref() == Some("1")
}

/// 从 `/api/sessions` 的响应里取运行中的会话 id（纯函数，便于单测）。
fn parse_running_ids(body: &[u8]) -> Vec<String> {
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    json.get("runningSessionIds")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn service_port(app: &AppHandle) -> Option<u16> {
    app.state::<crate::AppState>()
        .allowed_port
        .lock()
        .ok()
        .and_then(|port| *port)
}

/// 查 pi-web 当前运行中的会话。查不到（侧车没起/超时）就当作没有 —— 此时退出无风险。
fn running_sessions(app: &AppHandle) -> Vec<String> {
    let Some(port) = service_port(app) else {
        return Vec::new();
    };
    let url = format!("http://127.0.0.1:{port}/api/sessions");
    let output = std::process::Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "3",
            "-H",
            "Accept: application/json",
            &url,
        ])
        .output();
    match output {
        Ok(output) => parse_running_ids(&output.stdout),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_running_sessions() {
        let body = br#"{"sessions":[],"runningSessionIds":["a","b"],"awaitingInputSessionIds":[]}"#;
        assert_eq!(parse_running_ids(body), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn empty_or_broken_response_means_no_running_sessions() {
        assert!(parse_running_ids(br#"{"runningSessionIds":[]}"#).is_empty());
        assert!(parse_running_ids(b"not json").is_empty());
        assert!(parse_running_ids(br#"{}"#).is_empty());
    }
}
