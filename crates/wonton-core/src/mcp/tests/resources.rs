//! 读取插件资源：按大小上限，超限、上游出错和连不上分开报。

use super::super::McpServer;
use super::super::upstream::ReadError;
use super::*;

#[tokio::test]
async fn plugin_resources_are_read_with_a_size_limit() {
    let server = McpServer {
        id: PluginId::parse("app.example.notes").unwrap(),
        title: Default::default(),
        url: serve_notes(Notes::default()).await,
        socket: None,
        headers: Default::default(),
        tool_limits: Default::default(),
        tools: None,
    };
    let upstreams = Upstreams::new();
    let read = |uri: &'static str, limit| {
        let (upstreams, server) = (&upstreams, &server);
        async move { upstreams.read_resource(server, uri, limit).await }
    };
    assert_eq!(
        read("notes://text", 64).await.unwrap(),
        "一条笔记".as_bytes()
    );
    assert_eq!(read("notes://blob", 64).await.unwrap(), b"abc");
    assert!(matches!(
        read("notes://blob", 2).await,
        Err(ReadError::TooLarge)
    ));
    assert!(matches!(
        read("notes://missing", 64).await,
        Err(ReadError::Upstream(_))
    ));
    let dead = McpServer {
        url: "http://127.0.0.1:1/mcp".into(),
        ..server.clone()
    };
    let unreachable = upstreams.read_resource(&dead, "notes://text", 64).await;
    assert!(matches!(unreachable, Err(ReadError::Unreachable(_))));
}
