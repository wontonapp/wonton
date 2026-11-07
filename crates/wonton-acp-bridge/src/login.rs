use std::{future::Future, path::Path, sync::Arc, time::Duration};

use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use wonton_l10n::{Arg, Lang};

use crate::{
    acp::{AcpClient, auth::AuthMethod},
    error::{Detail, Error, Result, format_error},
    l10n::text as t,
};

mod terminal;

/// 登录过程中的聊天输出；实现是 Matrix 房间，测试用内存替身。
pub trait LoginSink {
    fn send_login_text(&self, text: &str) -> impl Future<Output = Result<()>> + Send;
    /// 撤回聊天里的一条消息；权限不够或网络失败时返回错误。
    fn redact(&self, event_id: &str, reason: &str) -> impl Future<Output = Result<()>> + Send;
}

/// 群里的登录命令转到和主人的私聊执行：命令回执留在群里，登录过程只在私聊。
pub struct LoginChat<'a, S: LoginSink> {
    /// 命令或回复所在的那个房间。
    pub current: &'a S,
    /// 和主人的私聊；当前房间不是私聊时，登录输出都发到这里。
    pub direct: Option<&'a S>,
    /// 当前房间不是和主人的私聊。
    pub group: bool,
}

/// 撤回已转交 Agent 的密钥消息时写的理由，房间里的人都能看到。
fn redact_reason(lang: Lang) -> String {
    t(lang, "login-redact-reason", &[])
}

const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;

pub enum LoginEvent {
    Url {
        url: String,
        message: String,
        answer: oneshot::Sender<&'static str>,
    },
    Output(String),
    Finished {
        reconnect: bool,
        result: Result<()>,
    },
}

pub struct LoginFlow {
    active: Option<ActiveLogin>,
    answer: Option<oneshot::Sender<&'static str>>,
    events: mpsc::Sender<LoginEvent>,
}

struct ActiveLogin {
    task: JoinHandle<()>,
    input: mpsc::Sender<String>,
    terminal: bool,
}

impl Drop for ActiveLogin {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LoginFlow {
    pub fn new() -> (Self, mpsc::Receiver<LoginEvent>) {
        let (events, receiver) = mpsc::channel(8);
        (
            Self {
                active: None,
                answer: None,
                events,
            },
            receiver,
        )
    }

    pub fn active(&self) -> bool {
        self.active.is_some()
    }

    /// 私聊里的收发：登录命令和回复都在同一个房间。
    // 各行其事的一组入参（消息、聊天目标、ACP、socket、忙状态、语言），再包一层也是逐个填。
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub async fn receive(
        &mut self,
        text: &str,
        event_id: &str,
        sink: &impl LoginSink,
        acp: &Arc<AcpClient>,
        socket: &Path,
        busy: bool,
        lang: Lang,
    ) -> Result<bool> {
        self.receive_from(
            text,
            event_id,
            &LoginChat {
                current: sink,
                direct: None,
                group: false,
            },
            acp,
            socket,
            busy,
            lang,
        )
        .await
    }

    // 各行其事的一组入参（消息、聊天目标、ACP、socket、忙状态、语言），再包一层也是逐个填。
    #[allow(clippy::too_many_arguments)]
    pub async fn receive_from(
        &mut self,
        text: &str,
        event_id: &str,
        chat: &LoginChat<'_, impl LoginSink>,
        acp: &Arc<AcpClient>,
        socket: &Path,
        busy: bool,
        lang: Lang,
    ) -> Result<bool> {
        let command = text
            .strip_prefix("/wonton login")
            .filter(|rest| rest.is_empty() || rest.starts_with(' '));
        if let Some(rest) = command {
            self.handle_command(rest, chat, acp, socket, busy, lang)
                .await?;
            return Ok(true);
        }
        // 群里的普通消息照常走 ACP；登录里的码和 key 在私聊里回。
        if chat.group {
            return Ok(false);
        }
        if self.answer.is_some() {
            chat.current
                .send_login_text(&t(lang, "login-answer-first", &[]))
                .await?;
            return Ok(true);
        }
        let Some(active) = &self.active else {
            return Ok(false);
        };
        if active.terminal {
            if text.len() > 16 * 1024 || text.contains(['\n', '\r', '\0']) {
                chat.current
                    .send_login_text(&t(lang, "login-code-limits", &[]))
                    .await?;
            } else if active.input.send(text.to_owned()).await.is_ok() {
                Self::redact_forwarded_message(event_id, chat.current, lang).await?;
            } else {
                chat.current
                    .send_login_text(&t(lang, "login-not-sent", &[]))
                    .await?;
            }
        } else {
            chat.current
                .send_login_text(&t(lang, "login-no-forward", &[]))
                .await?;
        }
        Ok(true)
    }

    async fn handle_command(
        &mut self,
        rest: &str,
        chat: &LoginChat<'_, impl LoginSink>,
        acp: &Arc<AcpClient>,
        socket: &Path,
        busy: bool,
        lang: Lang,
    ) -> Result<()> {
        let rest = rest.trim();
        // 群里发起时链接和码都不能留在群里：登录改到和主人的私聊里做。
        let sink = match (chat.group, chat.direct) {
            (true, None) => {
                chat.current
                    .send_login_text(&t(lang, "login-no-direct", &[]))
                    .await?;
                return Ok(());
            }
            (true, Some(direct)) => {
                chat.current
                    .send_login_text(&t(lang, "login-in-direct", &[]))
                    .await?;
                direct
            }
            (false, _) => chat.current,
        };
        if rest == "cancel" {
            self.answer.take().map(|answer| answer.send("cancel"));
            self.active.take();
            sink.send_login_text(&t(lang, "login-cancelled", &[]))
                .await?;
            return Ok(());
        }
        if matches!(rest, "accept" | "decline") {
            if let Some(answer) = self.answer.take() {
                let _ = answer.send(if rest == "accept" {
                    "accept"
                } else {
                    "decline"
                });
            } else {
                sink.send_login_text(&t(lang, "login-no-pending", &[]))
                    .await?;
            }
            return Ok(());
        }
        if self.active() || busy {
            sink.send_login_text(&t(lang, "login-finish-current", &[]))
                .await?;
        } else if rest.is_empty() {
            sink.send_login_text(&methods_text(acp.auth_methods(), lang))
                .await?;
        } else if let Some(method) = acp
            .auth_methods()
            .iter()
            .find(|method| method.id == rest && method.supported())
        {
            match self.start(method.clone(), acp.clone(), socket).await {
                Ok(()) => {
                    sink.send_login_text(&t(lang, "login-starting", &[]))
                        .await?
                }
                Err(error) => sink.send_login_text(&format_error(&error, lang)).await?,
            }
        } else if acp.auth_methods().iter().any(|method| method.id == rest) {
            sink.send_login_text(&t(lang, "login-unsupported-method", &[]))
                .await?;
        } else {
            sink.send_login_text(&t(lang, "login-unknown-method", &[]))
                .await?;
        }
        Ok(())
    }

    /// 密钥已转交 Agent，原文不能留在聊天记录里：撤回；撤不动就明说，让用户自己删。
    async fn redact_forwarded_message(
        event_id: &str,
        sink: &impl LoginSink,
        lang: Lang,
    ) -> Result<()> {
        if let Err(error) = sink.redact(event_id, &redact_reason(lang)).await {
            tracing::warn!(event_id, error = %error, "撤回登录密钥消息失败");
            sink.send_login_text(&t(lang, "login-secret-delete", &[]))
                .await?;
        }
        Ok(())
    }

    async fn start(
        &mut self,
        method: AuthMethod,
        acp: Arc<AcpClient>,
        socket: &Path,
    ) -> Result<()> {
        if !method.is_terminal() && !matches!(method.kind.as_deref(), None | Some("agent")) {
            return Err(Error::InvalidInput(Detail::msg(
                "login-auth-kind-unsupported",
            )));
        }
        let terminal = method.is_terminal();
        let (input, input_rx) = mpsc::channel(1);
        let events = self.events.clone();
        let socket = socket.to_path_buf();
        let task = tokio::spawn(async move {
            let operation = async {
                if terminal {
                    terminal::run(&socket, &method, input_rx, events.clone()).await
                } else {
                    authenticate(acp, &method.id, &events).await
                }
            };
            let result = tokio::time::timeout(LOGIN_TIMEOUT, operation)
                .await
                .unwrap_or_else(|_| Err(Error::Upstream(Detail::msg("login-timeout"))));
            let _ = events
                .send(LoginEvent::Finished {
                    reconnect: terminal && result.is_ok(),
                    result,
                })
                .await;
        });
        self.active = Some(ActiveLogin {
            task,
            input,
            terminal,
        });
        Ok(())
    }

    pub async fn event(
        &mut self,
        event: LoginEvent,
        sink: &impl LoginSink,
        lang: Lang,
    ) -> Result<bool> {
        // 取消后已排队的旧事件不能污染下一次登录。
        if self.active.is_none() {
            return Ok(false);
        }
        match event {
            LoginEvent::Url {
                url,
                message,
                answer,
            } => {
                sink.send_login_text(&t(
                    lang,
                    "login-link",
                    &[
                        ("message", Arg::from(message.as_str())),
                        ("url", Arg::from(url.as_str())),
                    ],
                ))
                .await?;
                self.answer = Some(answer);
            }
            LoginEvent::Output(text) => {
                sink.send_login_text(&text).await?;
            }
            LoginEvent::Finished { reconnect, result } => {
                self.answer.take();
                self.active.take();
                match result {
                    Ok(()) if !reconnect => {
                        sink.send_login_text(&t(lang, "login-done", &[])).await?;
                    }
                    Ok(()) => {}
                    Err(error) => {
                        sink.send_login_text(&format_error(&error, lang)).await?;
                    }
                }
                return Ok(reconnect);
            }
        }
        Ok(false)
    }
}

fn methods_text(methods: &[AuthMethod], lang: Lang) -> String {
    let methods: Vec<&AuthMethod> = methods.iter().filter(|method| method.supported()).collect();
    if methods.is_empty() {
        return t(lang, "login-no-methods", &[]);
    }
    let mut text = t(lang, "login-methods-header", &[]);
    for method in methods {
        text.push('\n');
        text.push_str(&t(
            lang,
            "login-method-line",
            &[
                ("name", Arg::from(method.name.as_str())),
                ("id", Arg::from(method.id.as_str())),
            ],
        ));
    }
    text
}

async fn authenticate(
    acp: Arc<AcpClient>,
    method: &str,
    events: &mpsc::Sender<LoginEvent>,
) -> Result<()> {
    let operation = acp.authenticate(method);
    tokio::pin!(operation);
    loop {
        tokio::select! {
            result = &mut operation => return result,
            Some(request) = acp.next_login_url() => {
                events.send(LoginEvent::Url { url: request.url, message: request.message, answer: request.answer })
                    .await
                .map_err(|_| Error::TransportUnavailable(Detail::msg("login-chat-closed")))?;
            }
        }
    }
}
