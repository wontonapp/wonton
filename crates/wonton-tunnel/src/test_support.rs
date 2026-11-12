//! 测试用的小工具。

use std::collections::HashSet;
use std::net::{Ipv4Addr, TcpListener, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// 挑号窗口的两端：Linux 的临时端口段以下、1024 以上。读不到临时端口段时用它。
const FALLBACK_WINDOW: (u16, u16) = (20_000, 32_767);
/// 比窗口还多个几次尝试就报错：窗口里全被占着时说明机器不对劲，没必要无限转。
const EXTRA_TRIES: usize = 4_096;

/// 已经发出去的号码，同一个进程里不重复。
static HANDED_OUT: Mutex<Option<HashSet<u16>>> = Mutex::new(None);
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// 回环上一个现在空着的端口号。
///
/// 给“先把号码挑好、过一会儿由别的进程或被测代码去绑”的测试用：映射到本机要求同号，被测方不能自己 bind 0。
/// 所以号码要避开内核的临时端口段（`/proc/sys/net/ipv4/ip_local_port_range`）：本机别的作业主动连出去时
/// 内核就从那一段分配源端口，挑好的号放掉到对方绑上之间会被抢走，映射起来是“端口被占用”。
///
/// TCP 和 UDP 都试绑确认空闲，进程内也不重复。放掉到对方绑上之间的竞争仍然存在，只能靠避开临时端口段减小。
pub fn unused_port() -> u16 {
    let (low, high) = window();
    let width = usize::from(high - low) + 1;
    let start = usize::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos()),
    )
    .unwrap_or(0)
        % width;
    let mut handed_out = lock();
    for _ in 0..width + EXTRA_TRIES {
        let offset = (start + CURSOR.fetch_add(1, Ordering::Relaxed)) % width;
        let port = low + u16::try_from(offset).expect("window fits u16");
        if handed_out
            .as_ref()
            .is_some_and(|taken| taken.contains(&port))
        {
            continue;
        }
        if is_free(port) {
            handed_out.get_or_insert_with(HashSet::new).insert(port);
            return port;
        }
    }
    panic!("no free port in {low}..={high}");
}

/// 可用的号码段：临时端口段以下，尽量从 `20000` 起，够不着时退到 `1024`；连 `1024` 都没有时用固定窗口。
fn window() -> (u16, u16) {
    let local = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|text| {
            let mut parts = text.split_whitespace();
            let low = parts.next()?.parse::<u16>().ok()?;
            let high = parts.next()?.parse::<u16>().ok()?;
            (low <= high).then_some(low)
        });
    window_for(local)
}

/// [`window`] 的判定本身：只看临时端口段的下界，好单测。窗口必须宽于一个号，去重时才有得挑。
fn window_for(low: Option<u16>) -> (u16, u16) {
    let Some(low) = low else {
        return FALLBACK_WINDOW;
    };
    let ceiling = low.saturating_sub(1);
    if ceiling > 20_000 {
        (20_000, ceiling)
    } else if ceiling > 1_024 {
        (1_024, ceiling)
    } else {
        FALLBACK_WINDOW
    }
}

/// 两种协议同时绑得上才算空闲。
fn is_free(port: u16) -> bool {
    let Ok(tcp) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) else {
        return false;
    };
    if UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).is_err() {
        return false;
    }
    drop(tcp);
    true
}

fn lock() -> MutexGuard<'static, Option<HashSet<u16>>> {
    HANDED_OUT.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{FALLBACK_WINDOW, unused_port, window_for};

    #[test]
    fn window_shrinks_below_the_ephemeral_range_but_never_to_one_port() {
        assert_eq!(window_for(Some(32_768)), (20_000, 32_767));
        assert_eq!(window_for(Some(20_002)), (20_000, 20_001));
        assert_eq!(window_for(Some(20_001)), (1_024, 20_000));
        assert_eq!(window_for(Some(15_000)), (1_024, 14_999));
        assert_eq!(window_for(Some(10_000)), (1_024, 9_999));
        assert_eq!(window_for(Some(1_025)), FALLBACK_WINDOW);
        assert_eq!(window_for(Some(1_024)), FALLBACK_WINDOW);
        assert_eq!(window_for(None), FALLBACK_WINDOW);
    }

    #[test]
    fn unused_ports_stay_out_of_the_ephemeral_range() {
        let (low, _) = super::window();
        let local_low = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
            .ok()
            .and_then(|text| text.split_whitespace().next()?.parse::<u16>().ok());
        let (first, second) = (unused_port(), unused_port());
        assert_ne!(first, second, "同一进程里不能发同一个号");
        for port in [first, second] {
            assert!(port >= 1_024, "{port} 是特权端口");
            if let Some(local_low) = local_low {
                assert!(port < local_low, "{port} 落在临时端口段 {local_low} 起");
            } else {
                assert!(
                    (FALLBACK_WINDOW.0..=FALLBACK_WINDOW.1).contains(&port),
                    "{port} 不在退路窗口里"
                );
            }
            assert!(
                std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
                "{port} 发出去时就不空闲"
            );
        }
        assert!(low < 32_768);
    }
}
