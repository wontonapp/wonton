//! 每个链接类别的往返。

use super::super::{MatrixTarget, RichLink, RichLinkKind};
use super::test_support::round_trip;

/// 契约：每个链接类别都能经 JSON 往返，壳收到的类别和地址都还在。
#[test]
fn every_link_kind_round_trips_through_json() {
    for kind in [
        RichLinkKind::External,
        RichLinkKind::Plugin {
            plugin_id: "app.grafana".into(),
        },
        RichLinkKind::Matrix {
            target: MatrixTarget::User {
                user_id: "@alice:home.example".into(),
            },
        },
        RichLinkKind::Matrix {
            target: MatrixTarget::Room {
                room_id: "!abc:home.example".into(),
                event_id: None,
            },
        },
        RichLinkKind::Matrix {
            target: MatrixTarget::Room {
                room_id: "#room:home.example".into(),
                event_id: Some("$event".into()),
            },
        },
        RichLinkKind::Email,
        RichLinkKind::Phone,
        RichLinkKind::Other,
    ] {
        let link = RichLink {
            url: "https://example.com/".into(),
            kind,
        };
        assert_eq!(round_trip(&link), link);
    }
}
