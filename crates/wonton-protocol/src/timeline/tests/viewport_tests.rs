//! 滚动位置和列表该停在哪的每个取值。

use super::super::{TimelineScroll, TimelineViewport};
use super::test_support::round_trip;

/// 契约：滚动位置双向都能经 JSON 往返，壳和核心对“在哪儿”的每个取值都认。
#[test]
fn every_scroll_and_viewport_round_trips_through_json() {
    for scroll in [TimelineScroll::Bottom, TimelineScroll::Away] {
        assert_eq!(round_trip(&scroll), scroll);
    }
    for viewport in [
        TimelineViewport::Bottom,
        TimelineViewport::Away { new_messages: 0 },
        TimelineViewport::Away { new_messages: 27 },
    ] {
        assert_eq!(round_trip(&viewport), viewport);
    }
    assert_eq!(
        serde_json::to_value(TimelineViewport::Away { new_messages: 2 }).unwrap(),
        serde_json::json!({"type": "away", "new_messages": 2})
    );
}
