//! 测试里的手机：一个客户端核心，连同它的数据目录。

use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use iroh::EndpointId;
use wonton_client::{
    Action, ChatAccount, ChatSync, ChatViewModel, ClientCore, CoreError, Network, PushMethod,
};

use super::Home;
use super::wait::WAIT;

/// 探针的间隔。探针只在本地看会话装没装好，不碰网络；隔一会儿再看是给换服务器、重连留插进来的空档。
const SIGNED_IN_PROBE: Duration = Duration::from_millis(25);
/// 探通之后还要把这一份会话盯住这么久，没被换掉、也没断开，才算真的就位：隧道刚抖过时会话会马上又被收起来。
const SIGNED_IN_SETTLE: Duration = Duration::from_secs(3);
/// 测试里的手机：一个客户端核心，连同它的数据目录。
pub struct Phone {
    pub(crate) core: Arc<ClientCore>,
}

impl Deref for Phone {
    type Target = ClientCore;

    fn deref(&self) -> &ClientCore {
        &self.core
    }
}
impl Phone {
    pub fn open(dir: &std::path::Path) -> Self {
        let core = ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap();
        Self { core }
    }

    pub fn device_id(&self) -> EndpointId {
        EndpointId::from_str(&self.home().device_id).unwrap()
    }

    pub fn connect(&self, home: &Home) {
        self.dispatch(Action::Home {
            action: wonton_protocol::HomeAction::ConnectHome {
                ticket: home.ticket(),
            },
        })
        .unwrap();
    }

    /// 等登录并同步起来，返回用户 ID 和设备 ID。
    ///
    /// 返回时调用方可以直接 `dispatch`：视图显示“已登录”、同步在跑还不够。换服务器或者隧道刚抖过时，旧视图会先留着，
    /// 新的一次登录正要把旧会话收起来，这时 `dispatch` 会回 `NotReady`。所以这里不只看视图，还拿一个不碰房间的小动作
    /// （[`Self::probe_dispatch`]）探到本地会话真的装好、并且安稳一阵没被换掉，才返回；中途断了就接着等下一次。
    pub async fn signed_in(&self) -> (String, String) {
        let deadline = tokio::time::Instant::now() + WAIT;
        let mut last = None;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                panic!(
                    "登录并可以 dispatch：{WAIT:?} 内没有等到，最后一次 dispatch：{last:?}\n当前：{:#?}",
                    self.chat()
                );
            }
            let Some(ids) = signed_in_ids(&self.chat()) else {
                tokio::time::sleep(SIGNED_IN_PROBE.min(deadline - now)).await;
                continue;
            };
            match self.probe_dispatch() {
                Ok(()) if self.settle(&ids, deadline).await => return ids,
                Ok(()) => {}
                Err(error) => last = Some(error.to_string()),
            }
            tokio::time::sleep(SIGNED_IN_PROBE.min(deadline - now)).await;
        }
    }

    /// 探一次本地会话装好没有：`CheckReady` 只取一次会话，没登录就 `NotReady`，登录了什么都不做。
    /// 不依赖房间，也不像改显示方式那样顺带写账号数据。
    fn probe_dispatch(&self) -> Result<(), CoreError> {
        self.dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::CheckReady,
        })
    }

    /// 等聊天会话真的能 dispatch：订阅时间线、进房间这类夹具入口先叫它，别在登录或者重连的窗口里吃 `NotReady`。
    ///
    /// 拿 [`wonton_protocol::ChatAction::CheckReady`] 探：没登录就 `NotReady`，登录了什么都不做。探通以后隔一个
    /// 间隔再探一次，躲开“刚探通就被下一次重连收走”；超时 `WAIT` 就带着最后一次错误和当前视图 panic。
    pub async fn wait_ready(&self) {
        let deadline = tokio::time::Instant::now() + WAIT;
        let mut last = None;
        loop {
            match self.probe_dispatch() {
                Ok(()) => {
                    tokio::time::sleep(SIGNED_IN_PROBE).await;
                    if self.probe_dispatch().is_ok() {
                        return;
                    }
                }
                Err(error) => {
                    let view = self.chat();
                    eprintln!(
                        "wait_ready: NotReady while account={:?} sync={:?} rooms={} err={error}",
                        view.account,
                        view.sync,
                        view.rooms.len()
                    );
                    last = Some(error.to_string());
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                panic!(
                    "聊天会话可以 dispatch 之前超时（{WAIT:?}，最后一次 dispatch：{last:?}）\n当前：{:#?}",
                    self.chat()
                );
            }
            tokio::time::sleep(SIGNED_IN_PROBE.min(deadline - now)).await;
        }
    }

    /// 这一份会话（同一用户、同一设备）要一直还在，直到窗口过完，才算稳下来。
    async fn settle(&self, ids: &(String, String), deadline: tokio::time::Instant) -> bool {
        let until = (tokio::time::Instant::now() + SIGNED_IN_SETTLE).min(deadline);
        while tokio::time::Instant::now() < until {
            if signed_in_ids(&self.chat()).as_ref() != Some(ids) {
                return false;
            }
            tokio::time::sleep(SIGNED_IN_PROBE.min(until - tokio::time::Instant::now())).await;
        }
        signed_in_ids(&self.chat()).as_ref() == Some(ids)
    }

    /// 这些测试登记推送端点，走的是 UnifiedPush：默认方式是保持连接（ADR-0003），先切过去才登记得了。
    pub async fn use_unified_push(&self) {
        self.dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushMethod {
                method: PushMethod::UnifiedPush,
            },
        })
        .unwrap();
    }
}

/// 视图里“已登录、同步在跑”的用户和设备；别的状态都算还没就位。
fn signed_in_ids(view: &ChatViewModel) -> Option<(String, String)> {
    match (&view.account, &view.sync) {
        (ChatAccount::SignedIn { user_id, device_id }, ChatSync::Running) => {
            Some((user_id.clone(), device_id.clone()))
        }
        _ => None,
    }
}
