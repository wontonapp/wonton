//! 测试驱动 FakeAgent 的几个入口（状态消息测试用）：发原始更新、按指定停止原因
//! 结束一轮、断开连接。放在子模块只为让 acp_test_support.rs 守住 800 行门禁。

use serde_json::{Value, json};

use super::{FakeAgent, notify, reply, send};

impl FakeAgent {
    /// 下一次 `_session/steering` 回这个结果（按调用顺序排）。
    pub(crate) async fn steer_reply(&self, outcome: Value) {
        self.link.steer_outcomes.lock().await.push_back(outcome);
    }

    /// 收到过的 `_session/steering` 请求的内容块，按顺序。
    pub(crate) async fn steers(&self) -> Vec<Value> {
        self.link
            .requests
            .lock()
            .await
            .iter()
            .filter(|request| request["method"] == "_session/steering")
            .map(|request| request["params"]["prompt"].clone())
            .collect()
    }

    /// 发一条原始的 `session/update` 通知：状态消息测试用它发工具类别、思考块、计划。
    pub(crate) async fn raw_update(&self, session: &str, update: Value) {
        let writer = self
            .link
            .writer
            .lock()
            .await
            .clone()
            .expect("还没有 ACP 连接");
        send(
            &writer,
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {"sessionId": session, "update": update}
            }),
        )
        .await;
    }
    /// 断开当前的 ACP 连接（Agent 进程崩溃）：挂起的请求会以连接已关错误结束。
    /// 连接任务里的读半边随 abort 释放，但 `link.writer` 和挂起轮次里还各攥着一份
    /// 写半边，不清掉文件描述符不关，客户端看不到 EOF。
    pub(crate) async fn drop_connection(&self) {
        let handlers = self.handlers.lock().await.drain(..).collect::<Vec<_>>();
        for handler in handlers {
            handler.abort();
        }
        *self.link.writer.lock().await = None;
        self.link.pending.lock().await.clear();
    }
    /// 让第 `index` 个挂起的一轮回复 `text` 并以给定 `stopReason` 结束。
    pub(crate) async fn respond_prompt_as(
        &self,
        index: usize,
        text: &str,
        stop_reason: &str,
    ) -> String {
        let pending = {
            let mut pending = self.link.pending.lock().await;
            pending.remove(index)
        };
        notify(
            &pending.writer,
            &pending.session,
            &format!("m-{index}"),
            text,
        )
        .await;
        reply(
            &pending.writer,
            &pending.id,
            json!({"stopReason": stop_reason}),
        )
        .await;
        pending.session
    }
}
