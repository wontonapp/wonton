use wonton_protocol::{
    BotRef, ChainCarrier, ChainError, ChainLimits, ChainRefusal, ConfirmationOrigin,
};

use super::{ChainLedgerError, ChainSessionSource, Chains};

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "caller".into(),
        bot: name.into(),
    }
}

fn origin() -> ConfirmationOrigin {
    ConfirmationOrigin {
        room_id: "!room".into(),
        thread_id: None,
    }
}

fn ledger() -> (tempfile::TempDir, Chains) {
    let dir = tempfile::tempdir().unwrap();
    let chains = Chains::open(
        dir.path().join("chains.json"),
        ChainLimits {
            max_depth: 10,
            ..ChainLimits::default()
        },
    )
    .unwrap();
    (dir, chains)
}

fn child(chains: &Chains, parent: &str, session: &str, name: &str) -> u32 {
    let expected = chains
        .preview_attach_from_session(parent, &bot(name))
        .unwrap();
    let actual = chains
        .register_session_from_parent(parent, session, bot(name), ChainSessionSource::HandOff)
        .unwrap();
    assert_eq!(actual, expected);
    actual
}

fn refusal(error: ChainLedgerError, expected: ChainRefusal) {
    assert!(
        matches!(error, ChainLedgerError::Rejected(ChainError::InvalidInput { refusal })
        if refusal == expected)
    );
}

#[test]
fn rejects_only_self_and_ancestors_not_siblings_or_repeated_targets() {
    let (_dir, chains) = ledger();
    let root = chains
        .open_acp_chain("user", origin(), bot("a"), "a")
        .unwrap();
    assert_eq!(child(&chains, "a", "b1", "b"), 2);
    refusal(
        chains
            .preview_attach_from_session("a", &bot("a"))
            .unwrap_err(),
        ChainRefusal::SelfCall,
    );
    refusal(
        chains
            .preview_attach_from_session("b1", &bot("a"))
            .unwrap_err(),
        ChainRefusal::Loop,
    );
    assert_eq!(child(&chains, "b1", "c", "c"), 3);
    for ancestor in ["a", "b"] {
        refusal(
            chains
                .preview_attach_from_session("c", &bot(ancestor))
                .unwrap_err(),
            ChainRefusal::Loop,
        );
    }
    assert_eq!(child(&chains, "a", "b2", "b"), 2);
    assert_eq!(child(&chains, "a", "d", "d"), 2);
    assert_eq!(child(&chains, "d", "b3", "b"), 3);
    chains.end_session("b1");
    chains.end_session("b2");
    assert_eq!(child(&chains, "a", "b4", "b"), 2);
    let row = chains.row(&root.chain).unwrap();
    assert_eq!(
        row.bots
            .iter()
            .filter(|entry| entry.bot == bot("b"))
            .count(),
        1
    );
    assert_eq!(
        row.bots
            .iter()
            .find(|entry| entry.bot == bot("b"))
            .unwrap()
            .layer,
        2
    );
    assert_eq!(chains.session_chain("b3").unwrap().layer, 3);
    assert_eq!(chains.session_chain("b3").unwrap().parents, vec!["d"]);
}

#[test]
fn ambiguous_callers_union_paths_but_exact_parent_does_not() {
    let (_dir, chains) = ledger();
    let root = chains
        .open_acp_chain("user", origin(), bot("a"), "a")
        .unwrap();
    child(&chains, "a", "x", "x");
    child(&chains, "x", "b-deep", "b");
    child(&chains, "a", "b-shallow", "b");
    refusal(
        chains
            .preview_attach(&root.chain, &bot("x"), Some(&bot("b")))
            .unwrap_err(),
        ChainRefusal::Loop,
    );
    assert_eq!(
        chains
            .preview_attach_from_session("b-shallow", &bot("x"))
            .unwrap(),
        3
    );
    assert_eq!(
        chains
            .preview_attach(&root.chain, &bot("c"), Some(&bot("b")))
            .unwrap(),
        4
    );
    chains
        .register_session(
            &root.chain,
            "c",
            bot("c"),
            Some(&bot("b")),
            ChainSessionSource::HandOff,
        )
        .unwrap();
    let mut parents = chains.session_chain("c").unwrap().parents;
    parents.sort();
    assert_eq!(parents, vec!["b-deep", "b-shallow"]);
    refusal(
        chains
            .preview_attach_from_session("c", &bot("x"))
            .unwrap_err(),
        ChainRefusal::Loop,
    );
    chains.end_session("b-deep");
    assert_eq!(
        chains
            .preview_attach(&root.chain, &bot("x"), Some(&bot("b")))
            .unwrap(),
        3
    );
    assert_eq!(
        chains.preview_attach(&root.chain, &bot("a"), None).unwrap(),
        1
    );
}

#[test]
fn reused_bridge_key_cannot_rewrite_an_existing_child_path() {
    let (_dir, chains) = ledger();
    let start = |name: &str, event: &str| wonton_protocol::ChainTurnReport::Start {
        session: "bridge".into(),
        room: "!room".into(),
        thread: None,
        sender: "user".into(),
        event_id: format!("{name}-{event}"),
    };
    chains
        .start_bridge_turn(bot("a"), start("a", "1"), None, "user".into())
        .unwrap();
    child(&chains, "bridge", "b", "b");
    chains.end_session("bridge");
    chains
        .start_bridge_turn(bot("x"), start("x", "2"), None, "user".into())
        .unwrap();
    refusal(
        chains
            .preview_attach_from_session("b", &bot("a"))
            .unwrap_err(),
        ChainRefusal::Loop,
    );
    assert_eq!(
        chains.preview_attach_from_session("b", &bot("x")).unwrap(),
        3
    );
}

fn connection(chains: &Chains, key: &str, target: &str) -> String {
    chains
        .open_acp_connection("user", origin(), bot(target), key)
        .unwrap()
        .chain
}

#[test]
fn carrier_uses_live_callers_and_depth_only_raises_the_parent_layer() {
    let (_dir, chains) = ledger();
    let root = chains
        .open_acp_chain("user", origin(), bot("a"), "a")
        .unwrap();
    child(&chains, "a", "b", "b");
    child(&chains, "b", "c", "c");
    connection(&chains, "conn", "d");
    let attach = |session, depth| {
        chains.attach_carrier(
            "conn",
            session,
            "caller",
            Some(&bot("b")),
            Some(&ChainCarrier {
                id: root.chain.clone(),
                depth,
            }),
        )
    };
    assert_eq!(attach("d-low", 0).unwrap().layer, 3);
    assert_eq!(attach("d-high", 5).unwrap().layer, 6);
    assert_eq!(attach("d-again", 1).unwrap().layer, 3);
    connection(&chains, "conn-x", "x");
    let ambiguous = chains
        .attach_carrier(
            "conn-x",
            "x",
            "caller",
            None,
            Some(&ChainCarrier {
                id: root.chain.clone(),
                depth: 0,
            }),
        )
        .unwrap();
    assert_eq!(ambiguous.layer, 7);
    refusal(
        chains
            .preview_attach_from_session("x", &bot("c"))
            .unwrap_err(),
        ChainRefusal::Loop,
    );
}

#[test]
fn placeholders_do_not_claim_membership_and_missing_carrier_uses_issued_root() {
    let (_dir, chains) = ledger();
    let issued = connection(&chains, "conn-a", "a");
    connection(&chains, "conn-b", "b");
    assert!(!chains.session_chain("conn-a").unwrap().live);
    let grant = chains
        .attach_carrier(
            "conn-b",
            "b",
            "caller",
            Some(&bot("a")),
            Some(&ChainCarrier {
                id: issued.clone(),
                depth: 2,
            }),
        )
        .unwrap();
    assert_ne!(grant.chain, issued);
    assert_eq!(grant.layer, 3);
    let root = chains
        .attach_carrier("conn-a", "a", "web", None, None)
        .unwrap();
    assert_eq!(root.chain, issued);
    assert_eq!(root.layer, 1);
    assert!(!chains.session_chain("a").unwrap().connection);
    assert!(chains.session_chain("a").unwrap().parents.is_empty());
}

#[test]
fn unknown_ended_nonmember_and_stopped_carriers_have_distinct_admission() {
    let (_dir, chains) = ledger();
    connection(&chains, "conn", "target");
    let live = chains
        .open_acp_chain("user", origin(), bot("a"), "a")
        .unwrap();
    let ended = chains
        .open_acp_chain("user", origin(), bot("e"), "e")
        .unwrap();
    chains.end_session("e");
    chains.end_chain(&ended.chain).unwrap();
    for (session, id, initiator) in [
        ("unknown", "unknown", bot("a")),
        ("ended", ended.chain.as_str(), bot("e")),
        ("nonmember", live.chain.as_str(), bot("outsider")),
    ] {
        let grant = chains
            .attach_carrier(
                "conn",
                session,
                "caller",
                Some(&initiator),
                Some(&ChainCarrier {
                    id: id.into(),
                    depth: 3,
                }),
            )
            .unwrap();
        assert_ne!(grant.chain, id);
        assert_eq!(grant.layer, 4);
        assert!(chains.session_chain(session).unwrap().parents.is_empty());
    }
    chains.stop(&live.chain).unwrap();
    let error = chains
        .attach_carrier(
            "conn",
            "stopped",
            "caller",
            Some(&bot("outsider")),
            Some(&ChainCarrier {
                id: live.chain,
                depth: 0,
            }),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ChainLedgerError::Rejected(ChainError::Stopped)
    ));
}

#[test]
fn depth_boundary_is_parent_based_and_never_wraps() {
    let (_dir, chains) = ledger();
    let root = chains
        .open_acp_chain("user", origin(), bot("a"), "a")
        .unwrap();
    connection(&chains, "conn", "b");
    let last = chains
        .attach_carrier(
            "conn",
            "last",
            "caller",
            Some(&bot("a")),
            Some(&ChainCarrier {
                id: root.chain.clone(),
                depth: 9,
            }),
        )
        .unwrap();
    assert_eq!(last.layer, 10);
    let error = chains
        .preview_attach_from_session("last", &bot("c"))
        .unwrap_err();
    assert!(matches!(
        error,
        ChainLedgerError::Rejected(ChainError::LimitReached {
            kind: wonton_protocol::ChainLimitKind::Depth,
        })
    ));
    for depth in [10, u32::MAX] {
        let error = chains
            .attach_carrier(
                "conn",
                "too-deep",
                "caller",
                Some(&bot("a")),
                Some(&ChainCarrier {
                    id: root.chain.clone(),
                    depth,
                }),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            ChainLedgerError::Rejected(ChainError::LimitReached {
                kind: wonton_protocol::ChainLimitKind::Depth,
            })
        ));
    }
    assert_eq!(child(&chains, "a", "sibling", "b"), 2);
}
