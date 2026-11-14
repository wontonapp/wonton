use std::process::Command;
use std::time::Duration;

use wonton_client::{Action, PairingFailure, PairingStatus};
use wonton_protocol::{JoinOutcome, Permissions, RoleDef, RoleTemplate, SettingsAction};
use wonton_tunnel::pair::RejectReason;

use super::{Home, Phone};
use crate::home::support::Gate;

struct AutoGate(Gate);

impl Drop for AutoGate {
    fn drop(&mut self) {
        self.0.open();
    }
}

#[test]
fn pairing_failure_message_refreshes_after_language_changes() {
    const CHILD: &str = "WONTON_PAIR_LANGUAGE_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pairing::language::pairing_failure_message_refreshes_after_language_changes",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated language test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::open(dir.path());
        let home = Home::start().await;
        set_language(&phone, "en");
        phone.pair(&home.code(Some(RejectReason::Expired))).unwrap();
        let (english, japanese) = assert_refresh(&phone, PairingFailure::Expired).await;
        assert_other_failures_refresh(&phone, &home).await;

        let gate = AutoGate(Gate::default());
        phone.test_set_bind_hook(Some(gate.0.hook()));
        phone.test_set_pair_deadlines(Duration::from_secs(600), Duration::from_secs(600));
        phone.pair(&home.code(Some(RejectReason::Expired))).unwrap();
        set_language(&phone, "ja");
        gate.0.open();
        let failed = phone
            .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
            .await;
        let Some(PairingStatus::Failed {
            reason, message, ..
        }) = failed.pairing
        else {
            unreachable!()
        };
        assert_eq!(reason, PairingFailure::Expired);
        assert_eq!(message, japanese);
        set_language(&phone, "en");
        let restored = phone.home().pairing;
        assert!(
            matches!(restored, Some(PairingStatus::Failed { message, .. }) if message == english)
        );
        phone.test_set_bind_hook(None);
        assert_setup_failure_refreshes().await;
    });
}

fn set_language(phone: &Phone, tag: &str) {
    phone
        .dispatch(Action::Settings {
            action: SettingsAction::SetLanguages {
                languages: vec![tag.to_owned()],
            },
        })
        .unwrap();
}

async fn assert_refresh(phone: &Phone, expected: PairingFailure) -> (String, String) {
    let first = phone
        .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
        .await;
    let Some(PairingStatus::Failed {
        reason,
        message,
        diagnostic,
    }) = first.pairing
    else {
        unreachable!()
    };
    assert_eq!(reason, expected);
    set_language(phone, "ja");
    let translated = tokio::time::timeout(Duration::from_secs(3), phone.wait_for(|view| {
        matches!(&view.pairing, Some(PairingStatus::Failed { message: current, .. }) if current != &message)
    })).await.expect("pairing failure message did not update after changing language");
    let Some(PairingStatus::Failed {
        reason: translated_reason,
        message: translated_message,
        diagnostic: translated_diagnostic,
    }) = translated.pairing
    else {
        unreachable!()
    };
    assert_eq!(translated_reason, reason);
    assert_eq!(translated_diagnostic, diagnostic);
    set_language(phone, "en");
    let restored = tokio::time::timeout(Duration::from_secs(3), phone.wait_for(|view| {
        matches!(&view.pairing, Some(PairingStatus::Failed { message: current, .. }) if current == &message)
    })).await.expect("pairing failure message did not return to the original language");
    let Some(PairingStatus::Failed {
        reason: restored_reason,
        diagnostic: restored_diagnostic,
        ..
    }) = restored.pairing
    else {
        unreachable!()
    };
    assert_eq!(restored_reason, reason);
    assert_eq!(restored_diagnostic, diagnostic);
    (message, translated_message)
}

async fn assert_other_failures_refresh(phone: &Phone, home: &Home) {
    phone.pair("wonton://pair?v=unsupported").unwrap();
    assert_refresh(phone, PairingFailure::Protocol).await;

    let gate = AutoGate(Gate::default());
    phone.test_set_bind_hook(Some(gate.0.hook()));
    phone.test_set_pair_deadlines(Duration::from_millis(300), Duration::from_secs(600));
    phone.pair(&home.code(None)).unwrap();
    let (english, japanese) = assert_refresh(phone, PairingFailure::TransportUnavailable).await;
    assert!(english.contains("0.3"));
    assert!(japanese.contains("0.3"));
    gate.0.open();
    phone.test_set_bind_hook(None);

    let gate = AutoGate(Gate::default());
    phone.test_set_bind_hook(Some(gate.0.hook()));
    phone.test_set_pair_deadlines(Duration::from_secs(600), Duration::from_millis(300));
    phone.pair(&home.code(None)).unwrap();
    let (english, japanese) = assert_refresh(phone, PairingFailure::TransportUnavailable).await;
    assert!(english.contains("0.3"));
    assert!(japanese.contains("0.3"));
    phone.test_set_bind_hook(None);

    let role = RoleDef {
        id: "child".to_owned(),
        name: "Member".to_owned(),
        template: Some(RoleTemplate::Child),
        permissions: Permissions::strictest(),
        people: 0,
    };
    phone
        .pair(&home.join_code(JoinOutcome::Pending, Some(role)))
        .unwrap();
    gate.0.open();
    let (english, japanese) = assert_refresh(phone, PairingFailure::PendingApproval).await;
    assert!(english.contains("Member"));
    assert!(japanese.contains("Member"));
}

async fn assert_setup_failure_refreshes() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start().await;
    *home.codes.slow.lock().unwrap() = 1;
    set_language(&phone, "en");
    phone.pair(&home.code(None)).unwrap();
    phone
        .wait_for(|view| {
            matches!(
                view.pairing,
                Some(PairingStatus::InProgress {
                    stage: wonton_client::PairingStage::Confirming,
                    ..
                })
            )
        })
        .await;
    std::fs::create_dir_all(dir.path().join("servers/1/home-ticket.json")).unwrap();
    assert_refresh(&phone, PairingFailure::Upstream).await;
    let Some(PairingStatus::Failed {
        diagnostic: Some(diagnostic),
        ..
    }) = phone.home().pairing
    else {
        panic!("post-pair setup failure must retain its diagnostic");
    };
    assert!(!diagnostic.is_empty());
}
