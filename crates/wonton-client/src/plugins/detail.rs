//! 详情页该显示什么：任务结束后关掉还是重取由核心判断，壳不自带一份。

use std::time::{Duration, Instant};

use wonton_protocol::{
    CoreError, PluginDetail, PluginDetailOutcome, PluginDetailRoute, PluginFailure,
    PluginOperation, PluginProblem, PluginTaskState,
};

use super::POLL_INTERVAL;
use super::PluginsLink;
use super::api::{READ_TIMEOUT, SOURCE_TIMEOUT, to_core_error};

/// 卸载的结果不确定时页面还开着：连不上家里的服务器就一直重取，最多这么久，恢复后认出没装就关。
const RETRY_TIMEOUT: Duration = Duration::from_secs(120);

impl PluginsLink {
    /// 取详情原始的问题：客户端核心要自己认出“插件没装”，不只是报错。
    pub(crate) async fn fetch_detail(
        &self,
        route: &PluginDetailRoute,
    ) -> Result<PluginDetail, PluginProblem> {
        let api = self.api()?;
        match route {
            PluginDetailRoute::Store { source } => {
                api.get(&["store", "plugin"], &[("source", source)], SOURCE_TIMEOUT)
                    .await
            }
            PluginDetailRoute::Installed { id } => {
                api.get(&["plugins", id], &[], READ_TIMEOUT).await
            }
        }
    }

    /// 打开详情页时的内容：插件已经卸掉时给 `Close`。
    pub(crate) async fn detail(
        &self,
        route: &PluginDetailRoute,
    ) -> Result<PluginDetailOutcome, CoreError> {
        match self.fetch_detail(route).await {
            Ok(detail) => Ok(PluginDetailOutcome::Detail { detail }),
            // 商店那一页的来源没有了还能重试；主屏上的插件没有了，这一页就没东西可看了。
            Err(problem)
                if matches!(route, PluginDetailRoute::Installed { .. })
                    && problem.reason == PluginFailure::NotFound =>
            {
                Ok(PluginDetailOutcome::Close)
            }
            Err(problem) => Err(to_core_error(problem)),
        }
    }

    /// 详情页上的任务刚做完：卸载确定做成了就直接关，否则重取。卸载失败或者等不到结果时，
    /// 重取发现插件已经不在，也关。重取时连不上家里的服务器就接着等，恢复后认出没装再关。
    pub(crate) async fn detail_after_task(
        &self,
        route: &PluginDetailRoute,
    ) -> Result<PluginDetailOutcome, CoreError> {
        if let PluginDetailRoute::Installed { id } = route
            && self.uninstalled(id)
        {
            return Ok(PluginDetailOutcome::Close);
        }
        let deadline = Instant::now() + RETRY_TIMEOUT;
        loop {
            let problem = match self.detail(route).await {
                Ok(outcome) => return Ok(outcome),
                Err(problem) => problem,
            };
            if !matches!(problem, CoreError::TransportUnavailable { .. })
                || Instant::now() >= deadline
            {
                return Err(problem);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    fn uninstalled(&self, id: &str) -> bool {
        self.view.borrow().tasks.iter().any(|task| {
            task.id == id
                && task.operation == PluginOperation::Uninstall
                && task.state == PluginTaskState::Succeeded
        })
    }
}
