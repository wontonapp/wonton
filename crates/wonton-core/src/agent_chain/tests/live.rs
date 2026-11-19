//! 列表每行的 `live`：App 的停止按钮只在链上有轮次在跑时出现。

use super::{bot, harness, start};

/// 开始一轮 → 在跑；这一轮报了结束 → 不在跑，列表版本跟着变（长轮询的 App 才收得到）；
/// 同一条链上还有别的轮次时仍算在跑；停链之后不算。
#[test]
fn a_row_is_live_only_while_a_round_is_running() {
    let h = harness();
    let hermes = bot("app.hermes", "owner");
    let grant = start(&h, &hermes, "s-1", "@alex:home.example", "$e1").expect("开链");
    let live = |chain: &str| h.chains.row(chain).expect("链在账本里").live;
    assert!(live(&grant.chain));
    assert!(h.chains.view().chains[0].live);

    let before = h.chains.revision();
    h.chains.end_session("s-1");
    assert!(!live(&grant.chain));
    assert!(h.chains.revision() > before);
    let settled = h.chains.revision();
    h.chains.end_session("s-1");
    assert_eq!(h.chains.revision(), settled);

    let next = start(&h, &hermes, "s-2", "@alex:home.example", "$e2").expect("开链");
    assert!(live(&next.chain));
    h.chains.stop(&next.chain).expect("停链");
    assert!(!live(&next.chain));
}
