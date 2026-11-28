use std::sync::Arc;

use wonton_protocol::AgentModelsViewModel;

use super::ClientCore;
use crate::AgentModelsSubscription;

#[uniffi::export]
impl ClientCore {
    /// 当前的模型页（ADR-0129），用于首帧渲染。
    pub fn agent_models(&self) -> AgentModelsViewModel {
        self.views().agent_models.borrow().clone()
    }

    /// 订阅模型页，语义同 [`subscribe_session_list`](ClientCore::subscribe_session_list)。
    pub fn subscribe_agent_models(&self) -> Arc<AgentModelsSubscription> {
        Arc::new(AgentModelsSubscription::new(
            self.views().agent_models.subscribe(),
        ))
    }
}
