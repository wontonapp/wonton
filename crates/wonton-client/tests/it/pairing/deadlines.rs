use std::time::Duration;

use iroh::EndpointId;
use wonton_client::{HomeStatus, PairingFailure};

use super::support::{Home, Phone, paired};

/// 测试里的每一段时限：留够冷启动的绑定和建连，慢回应还是它的两倍多。多线程运行时是必需的：假服务端
/// 用阻塞睡眠拖住回应，单线程运行时里客户端自己的计时器也会跟着停住。
const STAGE: Duration = Duration::from_secs(3);
const WATCH: Duration = Duration::from_secs(30);

/// 回应在超时之后才到（中继慢）：请求已经送到、服务端也登记了，客户端不能报失败——确认一次就配上。
/// 确认和第一次都赶不上才报失败，那时码已经用掉，回 `used`。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_after_the_deadline_still_pairs() {
    let home = Home::start().await;
    *home.codes.slow.lock().unwrap() = 1;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.test_set_pair_deadlines(STAGE, WATCH);
    phone.pair(&home.code(None)).unwrap();
    let view = phone.wait_for(paired).await;
    assert_eq!(
        view.server_id,
        Some(home.server.endpoint().id().to_string())
    );
    assert_eq!(*home.codes.names.lock().unwrap(), vec!["Alice 的 iPhone"]);
    // 存的是服务端地址，不是码：重启后直接连上。
    drop(phone);
    let restarted = Phone::open(dir.path());
    restarted
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
}

/// 建连那一段超时就不确认了：没有 0-RTT，握手没完成时请求一个字节都发不出去，服务端不可能登记，
/// 再问一次只会让死服务端多等一段。用一条拖慢第一个包的通道把握手拖过时限，断言只发生了一次交换。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connect_that_misses_the_stage_fails_without_a_second_exchange() {
    let home = Home::start().await;
    // 设备→服务端的包都晚 1.5 秒才转：握手赶不上 1 秒的时限，整段交换超时。
    let proxy = crate::home::support::SlowUdp::start_first(
        home.server.endpoint().bound_sockets()[0],
        1000,
        Duration::from_millis(1500),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.test_set_pair_deadlines(Duration::from_secs(1), WATCH);
    phone.pair(&home.code_at(proxy.addr())).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::TransportUnavailable, "{message}");
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    let device: EndpointId = phone.home().device_id.parse().unwrap();
    assert!(
        !home.codes.allowlist.contains(&device),
        "握手都没完成，服务端不该登记"
    );
}

/// 第一次和自动的确认都超过时限才报失败；服务端已经登记了这台设备，拿同一个码手动重试还能配上，
/// 不会停在“码已经用过”的死胡同里。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retrying_the_same_code_after_a_failure_pairs() {
    let home = Home::start().await;
    *home.codes.slow.lock().unwrap() = 2;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.test_set_pair_deadlines(STAGE, WATCH);
    let code = home.code(None);
    phone.pair(&code).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::TransportUnavailable, "{message}");
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    let device: EndpointId = phone.home().device_id.parse().unwrap();
    assert!(
        home.codes.allowlist.contains(&device),
        "服务端已经登记了这台设备"
    );

    // 手动重试本身也可能是冷启动：给它宽一点的一段，码已经不再慢，配上就完事。
    phone.test_set_pair_deadlines(Duration::from_secs(10), WATCH);
    phone.pair(&code).unwrap();
    phone.wait_for(paired).await;
}
