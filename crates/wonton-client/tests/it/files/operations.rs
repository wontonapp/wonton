//! 文件操作和播放地址：操作到核心并说清哪个文件，播放地址只认令牌。

use super::*;

#[tokio::test]
async fn operations_reach_the_core_and_failures_say_which_file() {
    let world = World::start().await;
    let file = world.core.lock().serve("a.jpg", b"abc", "v1");
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::SendFileToDevices { id: file.clone() },
        })
        .unwrap();
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::GrantFileToRoom {
                id: file.clone(),
                room: "!bot:home".to_owned(),
            },
        })
        .unwrap();
    let expected = file.to_string();
    world
        .wait_core(|core| core.sent.contains(&expected) && core.grants.len() == 1)
        .await;

    world.core.lock().delete_fails_with = Some(FileFailure::FileChanged);
    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_client::FileAction::DeleteFile { id: file.clone() },
        })
        .unwrap();
    let problem = world.wait_problem().await;
    assert_eq!(problem.operation, FileOperation::Delete);
    assert_eq!(problem.file, file);
    assert_eq!(problem.reason, FileFailure::FileChanged);

    world
        .phone()
        .dispatch(Action::Files {
            action: wonton_protocol::FileAction::DismissFileProblem,
        })
        .unwrap();
    assert_eq!(world.phone().transfers().problem, None);
}

#[tokio::test]
async fn the_player_address_serves_ranges_only_with_the_token() {
    let world = World::start().await;
    let bytes = b"0123456789";
    let file = world.core.lock().serve("a.mov", bytes, "v1");
    let url = world.phone().file_stream_url(file).await.unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    let ranged = client
        .get(&url)
        .header("Range", "bytes=2-5")
        .send()
        .await
        .unwrap();
    assert_eq!(ranged.status(), 206);
    assert_eq!(ranged.headers()["content-range"], "bytes 2-5/10");
    assert_eq!(ranged.bytes().await.unwrap().as_ref(), b"2345");

    let head = client.head(&url).send().await.unwrap();
    assert_eq!(head.status(), 200);
    assert_eq!(head.headers()["content-length"], "10");
    assert_eq!(head.headers()["accept-ranges"], "bytes");
    assert_eq!(head.headers()["x-content-type-options"], "nosniff");

    let token = parsed.path_segments().unwrap().next().unwrap();
    let wrong = url.replace(token, &"0".repeat(token.len()));
    assert_eq!(client.get(&wrong).send().await.unwrap().status(), 404);
    let requests = world.core.lock().ranges.len();
    assert_eq!(requests, 2, "令牌不对的请求不进隧道");
}
