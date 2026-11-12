//! 把会同步阻塞的异步调用挪出运行时的 worker。
//!
//! iroh 1.4.1 的 `Endpoint::bind` 在 await 里同步跑整段阻塞代码（`netmon::Monitor::new` →
//! `netdev::get_interfaces` 的 netlink 网卡枚举），中间不让出。在 worker 上 await 它，这个 worker 就被占住；
//! 同一个任务里包着它的 `tokio::time::timeout` 也轮不到。慢设备上（Android 模拟器里经 ndk-translation 跑的
//! arm64 包）实测卡过几十秒，配对的 30 秒时限一直不触发。

use std::future::Future;

use tokio::runtime::Handle;
use tokio::task::JoinError;

/// 在阻塞线程池里用 `Handle::block_on` 跑 `make()` 造出的 future，调用方只 await 一个 `JoinHandle`。
///
/// 选它而不是另开线程或运行时：`block_on` 进的是调用方的运行时，future 里 `tokio::spawn` 出去的后台任务
/// （iroh 的各个 actor）照常落在调用方的运行时上，跟着它的作用域一起停；阻塞段占的只是池里的一个线程，
/// 外层的时限、心跳和别的任务照常走。调用方超时放弃时，这个线程跑完后自己收尾，结果丢掉。
///
/// 必须在 tokio 运行时里调用。`make` 里 panic 时返回 `JoinError`。
pub async fn off_workers<F>(
    make: impl FnOnce() -> F + Send + 'static,
) -> Result<F::Output, JoinError>
where
    F: Future,
    F::Output: Send + 'static,
{
    let handle = Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(make())).await
}
