use std::{future::Future, path::Path, sync::Arc, time::Duration};

use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    acp::{AcpClient, auth::AuthMethod},
    error::{Error, Result},
};

mod terminal;

/// 登录过程中的聊天输出；实现是 Matrix 房间，测试用内存替身。
pub trait LoginSink {
    fn send_login_text(&self, text: &str) -> impl Future<Output = Result<()>> + Send;
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

    pub async fn receive(
        &mut self,
        text: &str,
        sink: &impl LoginSink,
        acp: &Arc<AcpClient>,
        socket: &Path,
        busy: bool,
    ) -> Result<bool> {
        let command = text
            .strip_prefix("/wonton login")
            .filter(|rest| rest.is_empty() || rest.starts_with(' '));
        if let Some(rest) = command {
            let rest = rest.trim();
            if rest == "cancel" {
                self.answer.take().map(|answer| answer.send("cancel"));
                self.active.take();
                sink.send_login_text("登录已取消。").await?;
                return Ok(true);
            }
            if matches!(rest, "accept" | "decline") {
                if let Some(answer) = self.answer.take() {
                    let _ = answer.send(if rest == "accept" {
                        "accept"
                    } else {
                        "decline"
                    });
                } else {
                    sink.send_login_text("没有等待回答的登录链接。").await?;
                }
                return Ok(true);
            }
            if self.active() || busy {
                sink.send_login_text("请先结束当前操作，再发起登录。")
                    .await?;
            } else if rest.is_empty() {
                sink.send_login_text(&methods_text(acp.auth_methods()))
                    .await?;
            } else if let Some(method) = acp
                .auth_methods()
                .iter()
                .find(|method| method.id == rest && method.supported())
            {
                match self.start(method.clone(), acp.clone(), socket).await {
                    Ok(()) => {
                        sink.send_login_text(
                            "正在发起 Agent 登录。取消请发送 /wonton login cancel。",
                        )
                        .await?
                    }
                    Err(error) => sink.send_login_text(&error.to_string()).await?,
                }
            } else if acp.auth_methods().iter().any(|method| method.id == rest) {
                sink.send_login_text("桥还不支持这种登录方式。").await?;
            } else {
                sink.send_login_text("Agent 没有提供这种登录方式；发送 /wonton login 查看。")
                    .await?;
            }
            return Ok(true);
        }
        if self.answer.is_some() {
            sink.send_login_text(
                "请先回复 /wonton login accept 或 /wonton login decline 处理登录链接。",
            )
            .await?;
            return Ok(true);
        }
        let Some(active) = &self.active else {
            return Ok(false);
        };
        if active.terminal {
            if text.len() > 16 * 1024 || text.contains(['\n', '\r', '\0']) {
                sink.send_login_text("授权码必须是单行文字，且不能超过 16 KiB。")
                    .await?;
            } else if active.input.send(text.to_owned()).await.is_err() {
                sink.send_login_text("登录已结束，这条消息没有发送给 Agent。")
                    .await?;
            }
        } else {
            sink.send_login_text(
                "登录期间不转发普通聊天；请在官方页面完成登录，或发送 /wonton login cancel。",
            )
            .await?;
        }
        Ok(true)
    }

    async fn start(
        &mut self,
        method: AuthMethod,
        acp: Arc<AcpClient>,
        socket: &Path,
    ) -> Result<()> {
        if !method.is_terminal() && !matches!(method.kind.as_deref(), None | Some("agent")) {
            return Err(Error::InvalidInput("不支持 Agent 提供的登录类型".into()));
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
                .unwrap_or_else(|_| Err(Error::Upstream("登录超时，请重新发起".into())));
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

    pub async fn event(&mut self, event: LoginEvent, sink: &impl LoginSink) -> Result<bool> {
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
                sink.send_login_text(&format!("Agent 登录：\n{message}\n{url}\n打开此链接登录请回复 /wonton login accept；拒绝请回复 /wonton login decline。" )).await?;
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
                        sink.send_login_text("Agent 登录完成。").await?;
                    }
                    Ok(()) => {}
                    Err(error) => {
                        sink.send_login_text(&error.to_string()).await?;
                    }
                }
                return Ok(reconnect);
            }
        }
        Ok(false)
    }
}

fn methods_text(methods: &[AuthMethod]) -> String {
    let methods: Vec<&AuthMethod> = methods.iter().filter(|method| method.supported()).collect();
    if methods.is_empty() {
        return "Agent 没有提供登录方式。".into();
    }
    let mut text = String::from("选择 Agent 登录方式：");
    for method in methods {
        text.push_str(&format!("\n{}：/wonton login {}", method.name, method.id));
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
                    .await.map_err(|_| Error::TransportUnavailable("登录聊天已关闭".into()))?;
            }
        }
    }
}
