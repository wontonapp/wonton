//! 排队的请求和它们的结果怎么落进 ViewModel。

use std::sync::Arc;

use wonton_protocol::{
    AgentModelsProblem, AgentModelsState, AgentModelsViewModel, BotRef, KeyCheck, KeyCheckResult,
    ModelCatalog, ModelConfirm, ModelList, ModelListState, ModelProviderPage, ModelSelection,
    ModelTest, ModelTestState, SecretString, SelectModel,
};

use super::{AgentModelsLink, lock, view};

pub(super) struct Job {
    pub(super) bot: BotRef,
    pub(super) op: Op,
    /// 壳点的要报；后台重取目录失败不盖掉正显示的页面。
    pub(super) report: bool,
}

#[derive(Debug)]
pub(super) enum Op {
    Catalog,
    Models {
        provider: String,
    },
    SaveKey {
        provider: String,
        key: SecretString,
        base_url: Option<String>,
    },
    Disconnect {
        provider: String,
    },
    LoginStart {
        provider: String,
        redirect: bool,
    },
    LoginFinish {
        session: String,
        callback_url: String,
    },
    LoginCancel {
        session: String,
    },
    Select(SelectModel),
    Test,
}

/// 隧道不通时每个请求都同样失败成这个原因，不必各写一遍。
macro_rules! ask {
    ($api:ident, $call:expr) => {
        match &$api {
            Ok($api) => $call.await,
            Err(problem) => Err(problem.clone()),
        }
    };
}

/// 点开的就是这家时返回它。
fn opened<'a>(
    view: &'a mut AgentModelsViewModel,
    bot: &BotRef,
    provider: &str,
) -> Option<&'a mut ModelProviderPage> {
    if view.bot.as_ref() != Some(bot) {
        return None;
    }
    view.provider
        .as_mut()
        .filter(|page| page.row.id == provider)
}

impl AgentModelsLink {
    pub(super) async fn run(self: &Arc<Self>, Job { bot, op, report }: Job) {
        let api = self.api();
        match op {
            Op::Catalog => {
                let result = ask!(api, api.catalog(&bot));
                self.catalog_done(&bot, result, report);
            }
            Op::Models { provider } => {
                let result = ask!(api, api.models(&bot, &provider));
                self.models_done(&bot, &provider, result);
            }
            Op::SaveKey {
                provider,
                key,
                base_url,
            } => {
                let result = ask!(api, api.save_key(&bot, &provider, key, base_url));
                self.key_done(&bot, &provider, result);
            }
            Op::Disconnect { provider } => {
                let result = ask!(api, api.disconnect(&bot, &provider));
                self.disconnect_done(&bot, &provider, result);
            }
            Op::LoginStart { provider, redirect } => {
                let result = ask!(api, api.login_start(&bot, &provider, redirect));
                self.login_started(&bot, &provider, redirect, result);
            }
            Op::LoginFinish {
                session,
                callback_url,
            } => {
                let result = ask!(api, api.login_finish(&bot, &session, callback_url));
                self.login_ended(&bot, &session, result);
            }
            Op::LoginCancel { session } => {
                if let Err(problem) = ask!(api, api.login_cancel(&bot, &session)) {
                    tracing::info!(reason = ?problem.reason, "model login cancel failed");
                }
            }
            Op::Select(select) => {
                let result = ask!(api, api.select(&bot, &select));
                self.select_done(&bot, select, result);
            }
            Op::Test => {
                let result = ask!(api, api.test(&bot));
                self.test_done(&bot, result);
            }
        }
    }

    fn catalog_done(
        &self,
        bot: &BotRef,
        result: Result<ModelCatalog, AgentModelsProblem>,
        report: bool,
    ) {
        if self.viewed().as_ref() != Some(bot) {
            return;
        }
        let catalog = match result {
            Ok(catalog) => catalog,
            Err(problem) => {
                tracing::info!(reason = ?problem.reason, "model catalog failed");
                self.view.send_modify(|view| {
                    if view.bot.as_ref() == Some(bot)
                        && (report || view.state != AgentModelsState::Ready)
                    {
                        view.state = AgentModelsState::Failed { problem };
                    }
                });
                return;
            }
        };
        lock(&self.local).catalog = Some((bot.clone(), catalog.clone()));
        let mut models = None;
        let mut gone = false;
        self.view.send_modify(|view| {
            if view.bot.as_ref() != Some(bot) {
                return;
            }
            (view.recommended, view.more) = view::rows(&catalog, &view.search);
            if view.current != catalog.current {
                view.current = catalog.current.clone();
                view.test = None;
            }
            view.state = AgentModelsState::Ready;
            let Some(page) = view.provider.as_mut() else {
                return;
            };
            match view::find(&catalog, &page.row.id) {
                Some(provider) => {
                    if view::refresh(page, provider) {
                        models = Some(page.row.id.clone());
                    }
                }
                // 插件不再列这家（别处删了自定义服务商）：关掉它的页面。
                None => gone = true,
            }
        });
        if gone {
            self.drop_login();
            self.view.send_modify(|view| view.provider = None);
        }
        if let Some(provider) = models {
            self.enqueue(bot.clone(), Op::Models { provider });
        }
    }

    fn models_done(
        &self,
        bot: &BotRef,
        provider: &str,
        result: Result<ModelList, AgentModelsProblem>,
    ) {
        self.view.send_modify(|view| {
            if let Some(page) = opened(view, bot, provider) {
                page.models = match result {
                    Ok(list) => ModelListState::Ready {
                        models: list.models,
                    },
                    Err(problem) => ModelListState::Failed { problem },
                };
            }
        });
    }

    /// 服务商认了或没法验证就照存了：重取目录（“已接好”）并取模型。拒了的没存，只显示结果。
    fn key_done(&self, bot: &BotRef, provider: &str, result: Result<KeyCheck, AgentModelsProblem>) {
        let mut saved = false;
        self.view.send_modify(|view| {
            let Some(page) = opened(view, bot, provider) else {
                return;
            };
            page.busy = false;
            match result {
                Ok(check) => {
                    saved = check.check != KeyCheckResult::Rejected;
                    if saved {
                        page.models = ModelListState::Loading;
                    }
                    page.key_check = Some(check);
                }
                Err(problem) => view.action_problem = Some(problem),
            }
        });
        if saved {
            self.job(bot.clone(), Op::Catalog, false);
            self.enqueue(
                bot.clone(),
                Op::Models {
                    provider: provider.to_owned(),
                },
            );
        }
    }

    fn disconnect_done(
        &self,
        bot: &BotRef,
        provider: &str,
        result: Result<(), AgentModelsProblem>,
    ) {
        let mut done = false;
        self.view.send_modify(|view| {
            let Some(page) = opened(view, bot, provider) else {
                return;
            };
            page.busy = false;
            match result {
                Ok(()) => {
                    page.key_check = None;
                    done = true;
                }
                Err(problem) => view.action_problem = Some(problem),
            }
        });
        if done {
            self.job(bot.clone(), Op::Catalog, false);
        }
    }

    fn select_done(
        &self,
        bot: &BotRef,
        select: SelectModel,
        result: Result<ModelSelection, AgentModelsProblem>,
    ) {
        let result = result.and_then(|selection| match &selection.current {
            Some(current) => current
                .check()
                .map(|()| selection.clone())
                .map_err(|problem| super::api::unreadable(problem.0)),
            None => Ok(selection),
        });
        self.view.send_modify(|view| {
            if view.bot.as_ref() != Some(bot) {
                return;
            }
            let page = view
                .provider
                .as_mut()
                .filter(|page| page.row.id == select.provider);
            match result {
                Ok(ModelSelection {
                    confirm: Some(message),
                    ..
                }) => {
                    if let Some(page) = page {
                        page.busy = false;
                        page.confirm = Some(ModelConfirm {
                            model: select.model,
                            message,
                        });
                    }
                }
                Ok(ModelSelection { current, .. }) => {
                    if let Some(page) = page {
                        page.busy = false;
                    }
                    if view.current != current {
                        view.current = current;
                        view.test = None;
                    }
                }
                Err(problem) => {
                    if let Some(page) = page {
                        page.busy = false;
                    }
                    view.action_problem = Some(problem);
                }
            }
        });
    }

    fn test_done(&self, bot: &BotRef, result: Result<ModelTest, AgentModelsProblem>) {
        self.view.send_modify(|view| {
            if view.bot.as_ref() == Some(bot) {
                view.test = Some(match result {
                    Ok(result) => ModelTestState::Done { result },
                    Err(problem) => ModelTestState::Failed { problem },
                });
            }
        });
    }
}
