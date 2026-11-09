//! 回环 IPv4 → IPv6 UDP 代理，挡住 QNT 宣告的原地址，确保被测流量始终经过损耗与延迟注入。

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::task::{JoinHandle, JoinSet};

#[derive(Clone, Copy, Default)]
struct Settings {
    loss_per_mille: u16,
    delay: Duration,
}

pub(super) struct Wire {
    addr: SocketAddr,
    settings: Arc<Mutex<Settings>>,
    task: JoinHandle<()>,
}

impl Wire {
    pub(super) async fn start(server: SocketAddr) -> Self {
        assert!(server.is_ipv6());
        let near = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
        let far = Arc::new(UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap());
        let addr = near.local_addr().unwrap();
        let settings = Arc::new(Mutex::new(Settings::default()));
        let task = tokio::spawn({
            let settings = settings.clone();
            async move {
                let mut client = None;
                let mut outgoing = JoinSet::new();
                let mut from_client = vec![0; 65_535];
                let mut from_server = vec![0; 65_535];
                let mut near_phase = 0;
                let mut far_phase = 0;
                loop {
                    tokio::select! {
                        got = near.recv_from(&mut from_client) => {
                            let (len, from) = got.unwrap();
                            client = Some(from);
                            forward(&mut outgoing, far.clone(), server, *settings.lock().unwrap(),
                                &mut near_phase, &from_client[..len]);
                        }
                        got = far.recv_from(&mut from_server) => {
                            let (len, from) = got.unwrap();
                            assert_eq!(from, server);
                            if let Some(to) = client {
                                forward(&mut outgoing, near.clone(), to, *settings.lock().unwrap(),
                                    &mut far_phase, &from_server[..len]);
                            }
                        }
                        result = outgoing.join_next(), if !outgoing.is_empty() => {
                            result.unwrap().unwrap();
                        }
                    }
                }
            }
        });
        Self {
            addr,
            settings,
            task,
        }
    }

    pub(super) fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub(super) fn set(&self, loss_per_mille: u16, delay: Duration) {
        assert!(loss_per_mille <= 1_000);
        *self.settings.lock().unwrap() = Settings {
            loss_per_mille,
            delay,
        };
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        // 转发任务归主循环的 JoinSet 所有，取消时连同等待延迟的包一起清理。
        self.task.abort();
    }
}

fn forward(
    pending: &mut JoinSet<()>,
    socket: Arc<UdpSocket>,
    to: SocketAddr,
    settings: Settings,
    phase: &mut u16,
    datagram: &[u8],
) {
    // 每个方向独立按比例丢弃，不让并发测试共享随机数状态。
    *phase += settings.loss_per_mille;
    if *phase >= 1_000 {
        *phase -= 1_000;
        return;
    }
    let datagram = datagram.to_vec();
    pending.spawn(async move {
        if !settings.delay.is_zero() {
            tokio::time::sleep(settings.delay).await;
        }
        socket.send_to(&datagram, to).await.unwrap();
    });
}
