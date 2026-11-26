use std::time::Duration;

use super::{STOP, seeded};

fn typing(out: &super::TypingOut) -> bool {
    super::lock(&out.state).typing
}

/// 5 秒没有新的输入就自己报“停了”。
#[tokio::test(start_paused = true)]
async fn quiet_for_five_seconds_stops_the_notice() {
    let out = seeded();
    out.set(true);
    assert!(typing(&out));
    tokio::time::advance(STOP - Duration::from_millis(1)).await;
    assert!(typing(&out));
    // 时间推进后计时任务不一定当拍跑完：小步睡，等状态真的变。
    tokio::time::timeout(STOP * 2, async {
        while typing(&out) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("过了 5 秒还在“在输入”");
}

/// 一直有输入就一直是“在输入”，计时从最后一下重新算。
#[tokio::test(start_paused = true)]
async fn continued_typing_keeps_the_notice_on() {
    let out = seeded();
    for _ in 0..4 {
        out.set(true);
        tokio::time::advance(Duration::from_secs(4)).await;
    }
    assert!(typing(&out));
    out.set(false);
    assert!(!typing(&out));
}
