//! 列表每行的 `live`：App 的停止按钮只在链上有轮次在跑时出现。最后一轮结束链就记成做完、
//! 从列表里下去，App 不会一直拿着一条早跑完的链显示在跑。

use wonton_protocol::AgentChainStatus;

use super::{bot, hand_off, harness, start};

/// 开始一轮 → 在跑；这一轮报了结束 → 不在跑，列表版本跟着变（长轮询的 App 才收得到），
/// 链也不再进列表；停链之后不算（也停得成）。
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
    assert!(h.chains.view().chains.is_empty(), "跑完的链不在列表里");
    let settled = h.chains.revision();
    h.chains.end_session("s-1");
    assert_eq!(h.chains.revision(), settled);

    let next = start(&h, &hermes, "s-2", "@alex:home.example", "$e2").expect("开链");
    assert!(live(&next.chain));
    h.chains.stop(&next.chain).expect("停链");
    assert!(!live(&next.chain));
    assert!(h.chains.view().chains.is_empty(), "停了的链不在列表里");
}

/// 链上还挂着别的轮次时，结束其中一轮仍算在跑；最后一轮结束才记成做完。
#[test]
fn a_chain_stays_live_while_another_round_runs() {
    let h = harness();
    let (a, b) = (bot("app.a", "owner"), bot("app.b", "owner"));
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    hand_off(&h, &chain, &a, &b, "s-2").expect("交给 B");

    h.chains.end_session("s-1");
    assert!(h.chains.row(&chain).expect("链在").live, "B 那轮还在跑");
    assert_eq!(h.chains.view().chains.len(), 1);

    h.chains.end_session("s-2");
    assert!(!h.chains.row(&chain).expect("链在").live);
    assert!(h.chains.view().chains.is_empty());
}

/// 到上限的链即使轮次都结束了也留在列表里等卡片回答（`live` 已经为假）；点了“再给一份”回到在跑。
#[test]
fn a_chain_at_the_limit_waits_in_the_list_without_a_round() {
    let h = harness();
    let a = bot("app.a", "owner");
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    h.chains.end_session("s-1");
    h.chains
        .record_tokens(&chain, &a, 2_000_000, None)
        .expect("记账到上限");
    let row = h.chains.row(&chain).expect("链在");
    assert_eq!(row.status, AgentChainStatus::LimitReached);
    assert!(!row.live);
    assert_eq!(h.chains.view().chains.len(), 1, "卡片还等着回答");

    h.chains.grant_another(&chain).expect("再给一份");
    assert_eq!(
        h.chains.row(&chain).expect("链在").status,
        AgentChainStatus::Running
    );
}
