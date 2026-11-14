use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use wonton_protocol::{Answer, CardAudience, CardChoice, ConfirmationCard, SecretValues};

use super::{Audience, Confirmations, Draft, Notifier, Recipients};
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;

struct Recorder {
    available: bool,
    recipients: Mutex<Vec<Recipients>>,
}

impl Recorder {
    fn available() -> Arc<Self> {
        Arc::new(Self {
            available: true,
            recipients: Mutex::new(Vec::new()),
        })
    }

    fn unavailable() -> Arc<Self> {
        Arc::new(Self {
            available: false,
            recipients: Mutex::new(Vec::new()),
        })
    }
}
impl Notifier for Recorder {
    fn post<'a>(
        &'a self,
        recipients: &'a Recipients,
        _card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            self.recipients.lock().unwrap().push(recipients.clone());
            if self.available {
                Ok(())
            } else {
                Err("没有已配对的设备".into())
            }
        }
        .boxed()
    }
}

fn draft() -> Draft {
    Draft {
        title: "安装应用".into(),
        lines: Vec::new(),
        choices: vec![
            CardChoice {
                answer: Answer::Approve,
                label: "安装".into(),
                choice_id: None,
            },
            CardChoice {
                answer: Answer::Reject,
                label: "拒绝".into(),
                choice_id: None,
            },
        ],
        scope: None,
        plugin_id: Some("app.example.notes".into()),
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    }
}
#[tokio::test]
async fn admin_cards_freeze_all_recipients_and_record_the_winning_user_id() {
    let notifier = Recorder::available();
    let confirmations = Arc::new(Confirmations::new(
        notifier.clone(),
        Duration::from_secs(30),
    ));
    let mut changed = confirmations.subscribe();
    let task = {
        let confirmations = confirmations.clone();
        tokio::spawn(async move {
            confirmations
                .ask_as(
                    &PluginId::parse("com.example.agent").unwrap(),
                    draft(),
                    Audience {
                        recipients: Recipients::Admins(vec!["alice".into(), "bob".into()]),
                        ..Audience::default()
                    },
                )
                .await
        })
    };
    changed.changed().await.unwrap();
    let card = confirmations.list(None).0.into_iter().next().unwrap();
    assert_eq!(card.audience, CardAudience::Admins);
    assert_eq!(confirmations.may_answer(&card.id, None), Some(true));
    assert_eq!(
        confirmations.may_answer(&card.id, Some("alice")),
        Some(true)
    );
    assert_eq!(confirmations.may_answer(&card.id, Some("bob")), Some(true));
    assert_eq!(
        confirmations.may_answer(&card.id, Some("carol")),
        Some(false)
    );

    confirmations
        .answer_with_secrets_by(
            &card.id,
            Answer::Approve,
            None,
            SecretValues::new(),
            Default::default(),
            Vec::new(),
            &Languages::from_header(None),
            Some("@alice:example.com"),
        )
        .unwrap();
    assert!(
        !confirmations
            .may_answer_as(&card.id, Some("bob"), Some("@bob:example.com"))
            .unwrap()
    );
    assert!(
        confirmations
            .may_answer_as(&card.id, Some("alice"), Some("@alice:example.com"))
            .unwrap()
    );
    assert_eq!(
        confirmations.list(Some("bob")).1[0].answered_by.as_deref(),
        Some("@alice:example.com")
    );
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn no_admin_device_is_reported_without_leaving_a_card() {
    let notifier = Recorder::unavailable();
    let confirmations = Arc::new(Confirmations::new(
        notifier.clone(),
        Duration::from_secs(30),
    ));
    let result = confirmations
        .ask_as(
            &PluginId::parse("com.example.agent").unwrap(),
            draft(),
            Audience {
                recipients: Recipients::Admins(vec!["alice".into(), "bob".into()]),
                ..Audience::default()
            },
        )
        .await;
    assert!(matches!(result, Err(super::Refusal::Unavailable(_))));
    assert!(confirmations.list(None).0.is_empty());
    assert!(confirmations.list(Some("alice")).0.is_empty());
}

#[tokio::test]
async fn a_keeper_card_keeps_the_single_recipient_behavior() {
    let notifier = Recorder::available();
    let confirmations = Arc::new(Confirmations::new(
        notifier.clone(),
        Duration::from_secs(30),
    ));
    let mut changed = confirmations.subscribe();
    let task = {
        let confirmations = confirmations.clone();
        tokio::spawn(async move {
            confirmations
                .ask_as(
                    &PluginId::parse("com.example.agent").unwrap(),
                    draft(),
                    Audience {
                        recipients: Recipients::Keeper(Some("alice".into())),
                        ..Audience::default()
                    },
                )
                .await
        })
    };
    changed.changed().await.unwrap();
    let card = confirmations.list(Some("alice")).0[0].clone();
    assert_eq!(card.audience, CardAudience::Keeper);
    assert_eq!(notifier.recipients.lock().unwrap().len(), 1);
    confirmations
        .answer_with_secrets_by(
            &card.id,
            Answer::Reject,
            None,
            SecretValues::new(),
            Default::default(),
            Vec::new(),
            &Languages::from_header(None),
            None,
        )
        .unwrap();
    assert!(matches!(task.await.unwrap(), Err(super::Refusal::Rejected)));
}
