//! 零依赖 HTTP 探活。
//!
//! pi-web 的 README 明确：Next 打印 "Ready" 早于请求处理器就绪，launcher 自己也是
//! 等 `GET /` 有应答才打开浏览器。所以这里同样用"任何 HTTP 应答即就绪"：
//! 只要状态行读得出来（200/302/401 都算），就算服务可用。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// 单次探测：TCP 连上 + 发一个最小 GET / + 读状态行。
pub fn http_answers(port: u16, timeout: Duration) -> bool {
    let addr: SocketAddr = match format!("127.0.0.1:{port}").parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let request = format!(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: pi-web-desktop\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut buf = [0u8; 16];
    match stream.read(&mut buf) {
        Ok(n) if n >= 5 => buf.starts_with(b"HTTP/"),
        _ => false,
    }
}

/// 轮询直到服务应答或超时。
pub fn wait_ready(port: u16, timeout: Duration, should_stop: &dyn Fn() -> bool) -> ReadyResult {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if should_stop() {
            return ReadyResult::Stopped;
        }
        if http_answers(port, Duration::from_millis(400)) {
            return ReadyResult::Ready(started.elapsed());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    ReadyResult::Timeout(started.elapsed())
}

pub enum ReadyResult {
    Ready(Duration),
    Timeout(Duration),
    Stopped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn notices_a_listening_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        assert!(http_answers(port, Duration::from_millis(800)));
    }

    #[test]
    fn rejects_a_closed_port() {
        // 拿一个刚释放的端口，基本可以认为没人监听
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!http_answers(port, Duration::from_millis(200)));
    }
}
