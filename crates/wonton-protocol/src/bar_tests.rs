use serde_json::json;

use super::*;
use crate::{Action, InboxCounts, InboxList, LayoutSettings, PageAction, PluginImage, TabRef};

fn rss() -> MyAppRef {
    MyAppRef::Plugin {
        plugin_id: "app.example.feed".into(),
    }
}

#[test]
fn bar_entries_have_a_stable_wire_shape() {
    assert_eq!(
        serde_json::to_value(BarEntry::Agent).unwrap(),
        json!({"type": "agent"})
    );
    assert_eq!(
        serde_json::to_value(BarEntry::Apps).unwrap(),
        json!({"type": "apps"})
    );
    assert_eq!(
        serde_json::to_value(BarEntry::App { app: rss() }).unwrap(),
        json!({"type": "app", "app": {"type": "plugin", "plugin_id": "app.example.feed"}})
    );
    let page = BarEntry::App {
        app: MyAppRef::Page {
            tab: TabRef::Url {
                url: "https://example.com/weather".into(),
            },
        },
    };
    let value = serde_json::to_value(&page).unwrap();
    assert_eq!(serde_json::from_value::<BarEntry>(value).unwrap(), page);
}

#[test]
fn a_layout_without_a_bar_gets_agent_and_apps_but_an_empty_bar_stays_empty() {
    let old: LayoutSettings = serde_json::from_value(json!({"my_apps": []})).unwrap();
    assert_eq!(old.bar, default_bar());
    assert_eq!(LayoutSettings::default().bar, default_bar());
    let emptied: LayoutSettings = serde_json::from_value(json!({"bar": []})).unwrap();
    assert!(emptied.bar.is_empty());
}

#[test]
fn bar_entries_from_a_newer_app_are_dropped_not_fatal() {
    let settings: LayoutSettings = serde_json::from_value(json!({
        "bar": [{"type": "apps"}, {"type": "inbox"}, {"type": "app", "app": {"type": "plugin", "plugin_id": "app.example.feed"}}]
    }))
    .unwrap();
    assert_eq!(
        settings.bar,
        vec![BarEntry::Apps, BarEntry::App { app: rss() }]
    );
}

#[test]
fn the_bar_holds_at_most_six_distinct_entries() {
    assert_eq!(check_bar(&default_bar()), Ok(()));
    assert_eq!(check_bar(&[]), Ok(()));
    let apps: Vec<BarEntry> = (0..MAX_BAR_ITEMS)
        .map(|index| BarEntry::App {
            app: MyAppRef::Plugin {
                plugin_id: format!("app.example.p{index}"),
            },
        })
        .collect();
    assert_eq!(check_bar(&apps), Ok(()));
    let mut seven = apps.clone();
    seven.push(BarEntry::Agent);
    assert_eq!(check_bar(&seven), Err(BarProblem::Full));
    assert_eq!(
        check_bar(&[
            BarEntry::Agent,
            BarEntry::App { app: rss() },
            BarEntry::App { app: rss() }
        ]),
        Err(BarProblem::Duplicate)
    );
    let unknown: BarProblem = serde_json::from_value(json!("too_wide")).unwrap();
    assert_eq!(unknown, BarProblem::Other);
}

#[test]
fn the_bar_view_model_round_trips_through_json() {
    let image = |path: &str| PluginImage { path: path.into() };
    let view = BarViewModel {
        items: vec![
            BarItem {
                entry: BarEntry::Agent,
                label: "Agent".into(),
                icon: BarIcon::Agent,
                badge: 2,
            },
            BarItem {
                entry: BarEntry::App { app: rss() },
                label: "RSS".into(),
                icon: BarIcon::App {
                    icon: Some(PluginIcon {
                        small: image("/_wonton/api/images/s.png"),
                        large: image("/_wonton/api/images/l.png"),
                    }),
                },
                badge: 0,
            },
        ],
        max_items: MAX_BAR_ITEMS as u32,
        full: true,
    };
    let value = serde_json::to_value(&view).unwrap();
    assert_eq!(serde_json::from_value::<BarViewModel>(value).unwrap(), view);
}

#[test]
fn bar_actions_round_trip_through_json() {
    for action in [
        PageAction::SelectBarItem {
            entry: BarEntry::Apps,
        },
        PageAction::AddBarItem { app: rss() },
        PageAction::RemoveBarItem {
            entry: BarEntry::Agent,
        },
        PageAction::MoveBarItem { from: 2, to: 0 },
    ] {
        let action = Action::Pages { action };
        let value = serde_json::to_value(&action).unwrap();
        assert_eq!(serde_json::from_value::<Action>(value).unwrap(), action);
    }
    assert_eq!(
        serde_json::to_value(PageAction::MoveBarItem { from: 2, to: 0 }).unwrap(),
        json!({"type": "move_bar_item", "from": 2, "to": 0})
    );
}

fn inbox(needing: u32, loading: bool, problem: Option<&str>) -> InboxViewModel {
    InboxViewModel {
        list: InboxList {
            items: Vec::new(),
            counts: InboxCounts {
                needing,
                unread_notices: 4,
            },
            revision: 1,
        },
        loading,
        problem: problem.map(str::to_owned),
    }
}

#[test]
fn the_bell_counts_only_what_waits_for_an_answer_and_admits_when_it_does_not_know() {
    // 通知不进铃铛的数字。
    assert_eq!(InboxBadge::of(&inbox(3, false, None)).needing, Some(3));
    assert_eq!(InboxBadge::of(&inbox(0, false, None)).needing, Some(0));
    // 没取到过、或最近一次没取成：不确定，不是 0。
    assert_eq!(InboxBadge::of(&inbox(0, true, None)).needing, None);
    assert_eq!(
        InboxBadge::of(&inbox(2, false, Some("offline"))).needing,
        None
    );
}
