//! 服务商登录：开始、轮询、结束。轮询直接发请求、不进动作队列：间隔由插件定（几秒一次），排在一次 60 秒的测试后面就跟不上了。

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    AgentModelsProblem, AgentModelsReason, BotRef, CoreError, LoginState, MODEL_LOGIN_REDIRECT_URI,
    ModelListState, ModelLogin, ModelLoginBrowser, ModelLoginProgress, ModelLoginState,
};

use super::jobs::Op;
use super::{AgentModelsLink, Login, invalid, lock};
use crate::l10n::{CATALOG, language};

/// 浏览器授权的回跳交回后插件还说“在等”时，改成轮询，最多等这么久。
const FINISH_GRACE: Duration = Duration::from_secs(120);

impl AgentModelsLink {
    pub(super) fn login_started(
        self: &Arc<Self>,
        bot: &BotRef,
        provider: &str,
        redirect: bool,
        result: Result<ModelLogin, AgentModelsProblem>,
    ) {
        let still_open = {
            let view = self.view.borrow();
            view.bot.as_ref() == Some(bot)
                && view
                    .provider
                    .as_ref()
                    .is_some_and(|page| page.row.id == provider)
        };
        let login = match result {
            Ok(login) if still_open => login,
            Ok(login) => {
                // 等结果时壳关了这家或换了 Bot：插件那边已经开了，取消掉。
                self.job(
                    bot.clone(),
                    Op::LoginCancel {
                        session: login.session,
                    },
                    false,
                );
                return;
            }
            Err(problem) => {
                self.view.send_modify(|view| {
                    if let Some(page) = view.provider.as_mut().filter(|_| still_open) {
                        page.busy = false;
                        view.action_problem = Some(problem);
                    }
                });
                return;
            }
        };
        lock(&self.local).login = Some(Login {
            bot: bot.clone(),
            provider: provider.to_owned(),
            session: login.session.clone(),
            redirect,
            state: redirect.then(|| query(&login.url, "state")).flatten(),
        });
        self.view.send_modify(|view| {
            if let Some(page) = view.provider.as_mut() {
                page.busy = false;
                page.login = Some(ModelLoginProgress {
                    url: login.url,
                    code: login.code,
                    browser: if redirect {
                        ModelLoginBrowser::Redirect
                    } else {
                        ModelLoginBrowser::Page
                    },
                });
            }
        });
        if !redirect {
            let deadline = Instant::now() + Duration::from_secs(login.expires_in.into());
            self.poll(bot, login.session, login.interval, deadline);
        }
    }

    /// 壳交回回跳网址。只认 App 的回跳地址，授权网址带了 `state` 的，回来的必须是同一个：别处发起的授权
    /// （登录劫持）不交给插件换凭据，这次登录按失败作废。插件换凭据时还要自己再校验一遍（规范要求）。
    pub(super) fn finish_login(
        self: &Arc<Self>,
        bot: BotRef,
        callback_url: String,
    ) -> Result<(), CoreError> {
        let login = lock(&self.local).login.clone();
        let login = login
            .filter(|login| login.redirect && login.bot == bot)
            .ok_or_else(invalid)?;
        self.busy()?;
        let ours = callback_url
            .strip_prefix(MODEL_LOGIN_REDIRECT_URI)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['?', '/', '#']));
        let state_ok = login
            .state
            .as_deref()
            .is_none_or(|state| query(&callback_url, "state").as_deref() == Some(state));
        if !(ours && state_ok) {
            tracing::info!("model login callback does not match this login; discarded");
            let failed = ModelLoginState {
                state: LoginState::Failed,
                message: Some(l10n!(CATALOG, language(), "models-login-callback-mismatch")),
            };
            self.login_ended(&bot, &login.session, Ok(failed));
            self.job(
                bot,
                Op::LoginCancel {
                    session: login.session,
                },
                false,
            );
            return Ok(());
        }
        self.enqueue(
            bot,
            Op::LoginFinish {
                session: login.session,
                callback_url,
            },
        );
        Ok(())
    }

    fn poll(self: &Arc<Self>, bot: &BotRef, session: String, interval: u32, deadline: Instant) {
        self.handle.spawn(poll(
            Arc::downgrade(self),
            bot.clone(),
            session,
            Duration::from_secs(interval.into()),
            deadline,
        ));
    }

    fn logging_in(&self, bot: &BotRef, session: &str) -> Option<Login> {
        lock(&self.local)
            .login
            .clone()
            .filter(|login| login.bot == *bot && login.session == session)
    }

    /// 一次登录有了结果。批准了就重取目录（“已接好”）和模型；别的结果留在页面上，直到再登录或关掉。
    pub(super) fn login_ended(
        self: &Arc<Self>,
        bot: &BotRef,
        session: &str,
        result: Result<ModelLoginState, AgentModelsProblem>,
    ) {
        let Some(login) = self.logging_in(bot, session) else {
            return;
        };
        if let Ok(ModelLoginState {
            state: LoginState::Pending,
            ..
        }) = result
        {
            self.view.send_modify(|view| {
                if let Some(page) = view.provider.as_mut() {
                    page.busy = false;
                }
            });
            self.poll(bot, login.session, 2, Instant::now() + FINISH_GRACE);
            return;
        }
        lock(&self.local).login = None;
        let approved = matches!(
            result,
            Ok(ModelLoginState {
                state: LoginState::Approved,
                ..
            })
        );
        self.view.send_modify(|view| {
            let Some(page) = view
                .provider
                .as_mut()
                .filter(|page| view.bot.as_ref() == Some(bot) && page.row.id == login.provider)
            else {
                return;
            };
            page.busy = false;
            page.login = None;
            match result {
                Ok(_) if approved => {
                    page.login_outcome = None;
                    page.models = ModelListState::Loading;
                }
                Ok(state) => page.login_outcome = Some(state),
                Err(problem) => view.action_problem = Some(problem),
            }
        });
        if approved {
            self.job(bot.clone(), Op::Catalog, false);
            self.enqueue(
                bot.clone(),
                Op::Models {
                    provider: login.provider,
                },
            );
        }
    }
}

/// 隔 `interval` 问一次插件，直到有结果、到期，或页面不再等这次登录（取消、关了、换了 Bot）。
async fn poll(
    link: Weak<AgentModelsLink>,
    bot: BotRef,
    session: String,
    interval: Duration,
    deadline: Instant,
) {
    loop {
        tokio::time::sleep(interval).await;
        let Some(link) = link.upgrade() else { return };
        if link.logging_in(&bot, &session).is_none() {
            return;
        }
        if Instant::now() >= deadline {
            let expired = ModelLoginState {
                state: LoginState::Expired,
                message: None,
            };
            link.login_ended(&bot, &session, Ok(expired));
            link.job(bot, Op::LoginCancel { session }, false);
            return;
        }
        let result = match link.api() {
            Ok(api) => api.login_poll(&bot, &session).await,
            Err(problem) => Err(problem),
        };
        match result {
            Ok(ModelLoginState {
                state: LoginState::Pending,
                ..
            }) => {}
            // 网络一时断了：到期前接着问，人在服务商那边点的批准不会因此丢掉。
            Err(problem) if problem.reason == AgentModelsReason::Transport => {
                tracing::info!("model login poll could not reach home; retrying");
            }
            result => {
                link.login_ended(&bot, &session, result);
                return;
            }
        }
    }
}

/// 网址查询串里某一项的值；网址读不懂或没有这一项时为空。
fn query(url: &str, name: &str) -> Option<String> {
    Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}
