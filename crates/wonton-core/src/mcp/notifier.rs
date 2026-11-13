//! 把新卡片通知给收件人的设备（ADR-0016 的“推送”）：核心直接发 Web Push，不进任何 Matrix 房间。
//! 之后的状态变化只改确认表，设备从“等你确认”列表的长轮询里知道，这里只管开卡的那一条。

use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use wonton_protocol::{CardChoice, ConfirmationCard, DeviceConfirmationPush};

use super::confirm::{Notifier, Recipients};
use crate::push::Push;

/// 开卡时给所有冻结收件人名下的每台已配对设备发确认推送。
pub struct DevicePush {
    push: Arc<Push>,
}

impl DevicePush {
    pub fn new(push: Arc<Push>) -> Self {
        Self { push }
    }
}

impl Notifier for DevicePush {
    fn post<'a>(
        &'a self,
        recipients: &'a Recipients,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            let confirmation = DeviceConfirmationPush {
                id: card.id.clone(),
                title: card.title.clone(),
                expires_at_ms: card.expires_at_ms,
                origin: card.origin.clone(),
                choices: button_choices(card),
            };
            let people: Vec<Option<&str>> = match recipients {
                Recipients::Keeper(person) => vec![person.as_deref()],
                Recipients::Admins(admins) => std::iter::once(None)
                    .chain(admins.iter().map(|admin| Some(admin.as_str())))
                    .collect(),
            };
            let mut paired = false;
            for person in people {
                if !self.push.has_devices(person) {
                    continue;
                }
                paired = true;
                if let Err(error) = self.push.send_confirmation(person, &confirmation).await {
                    tracing::warn!(
                        confirmation = %card.id,
                        person = person.unwrap_or("owner"),
                        %error,
                        "确认推送没发出去，App 靠列表取卡片"
                    );
                }
            }
            if paired {
                Ok(())
            } else {
                Err("所有收件人都没有已配对的设备，没人能回答确认".to_owned())
            }
        }
        .boxed()
    }
}

/// 通知上能画的按钮（ADR-0101 §1）：和卡片同一组，取前 3 个（推送载荷要小）。要填密钥或设置的卡片一个都不给，
/// 表单塞不进通知，点开进 App 填。
fn button_choices(card: &ConfirmationCard) -> Vec<CardChoice> {
    if !card.secret_inputs.is_empty() || card.settings.is_some() {
        return Vec::new();
    }
    card.choices.iter().take(3).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{Answer, ConfirmationState, SecretInput, SettingsForm};

    fn card(choices: Vec<CardChoice>) -> ConfirmationCard {
        ConfirmationCard {
            id: "card".into(),
            state: ConfirmationState::Pending,
            title: "Hermes 想安装 Grafana".into(),
            lines: Vec::new(),
            choices,
            expires_at_ms: 1,
            window: None,
            plugin_id: None,
            secret_inputs: Vec::new(),
            settings: None,
            requester: None,
            category: None,
            origin: None,
            audience: Default::default(),
            answered_by: None,
        }
    }

    fn choice(answer: Answer) -> CardChoice {
        CardChoice {
            answer,
            label: format!("{answer:?}"),
            choice_id: None,
        }
    }

    /// 通知上只画前 3 个按钮，标签和卡片上是同一批。
    #[test]
    fn at_most_three_buttons_go_into_the_push() {
        let buttons = button_choices(&card((0..5).map(|_| choice(Answer::Approve)).collect()));
        assert_eq!(buttons.len(), 3);
    }

    /// 表单塞不进通知：要密钥或要设置的卡片不给按钮，点开进 App 填（ADR-0101 §1）。
    #[test]
    fn cards_with_a_form_have_no_buttons() {
        let mut with_secret = card(vec![choice(Answer::Approve)]);
        with_secret.secret_inputs = vec![SecretInput {
            name: "key".into(),
            title: "密钥".into(),
            description: None,
            url: None,
            required: true,
        }];
        assert!(button_choices(&with_secret).is_empty());

        let mut with_settings = card(vec![choice(Answer::Approve)]);
        with_settings.settings = Some(SettingsForm { fields: Vec::new() });
        assert!(button_choices(&with_settings).is_empty());
    }
}
