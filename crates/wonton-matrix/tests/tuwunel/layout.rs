//! 账号数据：一台设备存的值，另一台设备第一次同步就能读到。

use std::time::{Duration, Instant};

use serde_json::json;
use wonton_matrix::testkit::{TestServer, unique};

const WAIT: Duration = Duration::from_secs(20);

#[tokio::test]
#[ignore = "需要 Docker，真实 Tuwunel 验证账号数据跟着账号走"]
async fn account_data_written_by_another_device_arrives_on_first_sync() {
    let server = TestServer::start().await;
    let localpart = unique("owner");
    let first = server.login(&localpart, "First").await;
    first
        .update_chat_layout(|content| {
            content.insert("default".to_owned(), json!({"form": "document"}));
        })
        .await
        .unwrap();
    drop(first);

    let second = server.login(&localpart, "Second").await;
    second.start_sync().await.unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(content) = second.chat_layout().await.unwrap() {
            assert_eq!(
                serde_json::to_value(content).unwrap(),
                json!({"default": {"form": "document"}})
            );
            return;
        }
        assert!(Instant::now() < deadline, "第一次同步没带到账号数据");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
