//! 扣下客户端发出去的 to-device 请求：回 200 让客户端当已送达继续走，请求原样记在手里，`replay` 再发给
//! 服务器，像它晚到一步。测试用它把“消息先到、房间密钥晚到”造出来；其余请求照常转发。

use std::{
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

/// 客户端经它连服务器。`hold(true)` 起，发往服务器的 to-device 请求（房间密钥走这里）扣在手里、回 200
/// （客户端当成已送达），`replay` 把记下的请求原样发给服务器——密钥这就到了，晚于客户端的等待上限。
pub struct ToDeviceHold {
    url: String,
    target: String,
    held: Arc<AtomicBool>,
    recorded: Arc<Mutex<Vec<Vec<u8>>>>,
    _accepting: JoinHandle<()>,
}

impl ToDeviceHold {
    /// 在本机空闲端口起中转，转发到 `target`（`http://主机:端口`）。
    pub async fn start(target: &str) -> Self {
        let target = target
            .strip_prefix("http://")
            .expect("只转发 http 地址")
            .trim_end_matches('/')
            .to_owned();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("中转监听本机端口");
        let url = format!("http://{}", listener.local_addr().expect("中转的地址"));
        let held = Arc::new(AtomicBool::new(false));
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let accepting = {
            let target = target.clone();
            let held = held.clone();
            let recorded = recorded.clone();
            tokio::spawn(async move {
                while let Ok((tcp, _)) = listener.accept().await {
                    tokio::spawn(serve(tcp, target.clone(), held.clone(), recorded.clone()));
                }
            })
        };
        Self {
            url,
            target,
            held,
            recorded,
            _accepting: accepting,
        }
    }

    /// 客户端连这个地址。
    pub fn url(&self) -> &str {
        &self.url
    }

    /// 开关扣下；开着时 to-device 请求记下并回 200，其余照常转发。
    pub fn hold(&self, hold: bool) {
        self.held.store(hold, Ordering::SeqCst);
    }

    /// 把记下的请求原样发给服务器，一个不落。回应答读完就扔：交付的是请求本身。
    pub async fn replay(&self) {
        let requests: Vec<Vec<u8>> =
            std::mem::take(&mut *self.recorded.lock().expect("中转的记录锁"));
        for request in requests {
            let Ok(mut upstream) = TcpStream::connect(&self.target).await else {
                tracing::warn!("to-device 重放连不上服务器，这条密钥丢了");
                continue;
            };
            if upstream.write_all(&request).await.is_err() {
                tracing::warn!("to-device 重放没写进去，这条密钥丢了");
                continue;
            }
            // 读完应答头（有 content-length 连体读完），连接就算干净地结束了。
            if let Some(head) = read_head(&mut upstream).await {
                let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
                if let Some(body) = read_body(&mut upstream, &text).await {
                    drop(body);
                }
            }
        }
    }
}

/// 一条连接只走一个请求：读下请求头，to-device 且扣着就记下回 200；其余连同体一起转给服务器，回应答
/// 原样搬回来后两边都关。转发时给请求写上 `connection: close`，逼客户端下个请求另起一条连接——不然
/// keep-alive 连接会变成透明管道，第二个请求（密钥就走在里面）绕过扣下的判断直达服务器。
async fn serve(
    mut tcp: TcpStream,
    target: String,
    held: Arc<AtomicBool>,
    recorded: Arc<Mutex<Vec<Vec<u8>>>>,
) {
    let Some(head) = read_head(&mut tcp).await else {
        return;
    };
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    if text
        .lines()
        .next()
        .is_some_and(|line| line.contains("sendtodevice"))
        && held.load(Ordering::SeqCst)
    {
        // 扣下：读完请求体（重放要原样），回 200 让客户端当已送达继续走。
        let mut request = head;
        if let Some(body) = read_body(&mut tcp, &text).await {
            request.extend_from_slice(&body);
        }
        recorded.lock().expect("中转的记录锁").push(request);
        let response = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\
                        connection: close\r\n\r\n{}";
        tcp.write_all(response.as_bytes()).await.ok();
        tcp.shutdown().await.ok();
        return;
    }
    let Ok(mut upstream) = TcpStream::connect(&target).await else {
        return;
    };
    if upstream.write_all(&close_after(&head)).await.is_err() {
        return;
    }
    if let Some(body) = read_body(&mut tcp, &text).await {
        upstream.write_all(&body).await.ok();
    }
    tokio::io::copy_bidirectional(&mut tcp, &mut upstream)
        .await
        .ok();
}

/// 请求头改写：去掉已有的 `connection:` 头、在请求行后补上 `connection: close`。
fn close_after(head: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let mut rewritten = Vec::with_capacity(head.len() + 20);
    let mut lines = text.split_inclusive("\r\n");
    if let Some(request_line) = lines.next() {
        rewritten.extend_from_slice(request_line.as_bytes());
    }
    rewritten.extend_from_slice(b"connection: close\r\n");
    for line in lines {
        if line.len() < 12 || !line[..11].eq_ignore_ascii_case("connection:") {
            rewritten.extend_from_slice(line.as_bytes());
        }
    }
    rewritten
}

/// 读到请求头结束（`\r\n\r\n`）；连接先断就算了。
async fn read_head(tcp: &mut TcpStream) -> Option<Vec<u8>> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        if tcp.read(&mut byte).await.unwrap_or(0) == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    Some(head)
}

/// 按 content-length 读请求体；不认识的长短当作没有体。
async fn read_body(tcp: &mut TcpStream, head: &str) -> Option<Vec<u8>> {
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    tcp.read_exact(&mut body).await.ok()?;
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::close_after;

    #[test]
    fn close_after_replaces_the_connection_header() {
        let head = b"PUT /sendToDevice/m.room.encrypted/t1 HTTP/1.1\r\n\
                     host: proxy\r\n\
                     connection: keep-alive\r\n\
                     authorization: Bearer t\r\n\
                     content-length: 2\r\n\
                     \r\n{}";
        let rewritten = String::from_utf8(close_after(head)).unwrap();
        assert_eq!(
            rewritten,
            "PUT /sendToDevice/m.room.encrypted/t1 HTTP/1.1\r\n\
             connection: close\r\n\
             host: proxy\r\n\
             authorization: Bearer t\r\n\
             content-length: 2\r\n\
             \r\n{}"
        );
    }
}
