use super::*;
use iroh::protocol::{AcceptError, ProtocolHandler};

#[tokio::test]
async fn malformed_truncated_and_oversize_requests_do_not_reach_authentication() {
    let mut setup = Setup::new(None).await;
    let cases = [
        (frame::MAX_REQUEST + 1, Vec::new()),
        (u32::MAX as usize, Vec::new()),
        (1, b"{".to_vec()),
        (32, b"{}".to_vec()),
    ];
    for (length, payload) in cases {
        let conn = setup.raw().await;
        let (mut send, _recv) = conn.open_bi().await.unwrap();
        send.write_all(&[frame::CONTROL]).await.unwrap();
        send.write_all(&(length as u32).to_be_bytes())
            .await
            .unwrap();
        send.write_all(&payload).await.unwrap();
        send.finish().unwrap();
        timeout(WAIT, conn.closed()).await.unwrap();
    }
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 0);
    assert!(setup.forwards.try_recv().is_err());
    setup.stop().await;
}

#[tokio::test]
async fn exact_maximum_request_is_accepted_but_one_byte_more_is_not() {
    let setup = Setup::new(None).await;
    let conn = setup.raw().await;
    let mut payload = serde_json::to_vec(&ShareRequest::Authenticate {
        openid: token("valid-credential"),
    })
    .unwrap();
    payload.resize(frame::MAX_REQUEST, b' ');
    assert_eq!(payload.len(), frame::MAX_REQUEST);
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&[frame::CONTROL]).await.unwrap();
    send.write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .unwrap();
    send.write_all(&payload).await.unwrap();
    send.finish().unwrap();
    let reply: ShareReply = timeout(WAIT, frame::read(&mut recv, frame::MAX_REPLY))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(reply, ShareReply::Shares { .. }));
    conn.close(0u32.into(), b"done");
    let conn = setup.raw().await;
    let (mut send, _) = conn.open_bi().await.unwrap();
    send.write_all(&[frame::CONTROL]).await.unwrap();
    send.write_all(&((frame::MAX_REQUEST + 1) as u32).to_be_bytes())
        .await
        .unwrap();
    timeout(WAIT, conn.closed()).await.unwrap();
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 1);
    setup.stop().await;
}

#[derive(Debug)]
struct ChangingServer;

impl ProtocolHandler for ChangingServer {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        for user in [USER, "@replacement:identity.test"] {
            let (mut send, mut recv) = conn.accept_bi().await.unwrap();
            let mut tag = [0];
            recv.read_exact(&mut tag).await.unwrap();
            let _: ShareRequest = frame::read(&mut recv, frame::MAX_REQUEST).await.unwrap();
            frame::write(
                &mut send,
                &ShareReply::Shares {
                    shares: shares(user),
                },
                frame::MAX_REPLY,
            )
            .await
            .unwrap();
        }
        conn.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn client_rejects_identity_replacement_from_a_hostile_server() {
    let router = Router::builder(endpoint().await)
        .accept(SHARE_ALPN, ChangingServer)
        .spawn();
    let phone = endpoint().await;
    let client = ShareClient::connect(
        phone.clone(),
        router.endpoint().addr(),
        token("valid-credential"),
    )
    .await
    .unwrap();
    assert_eq!(client.list().await, Err(ShareError::Protocol));
    assert_eq!(client.user_id(), USER);
    assert!(client.is_closed());
    phone.close().await;
    router.shutdown().await.unwrap();
}

#[derive(Debug)]
struct OversizeReply;
impl ProtocolHandler for OversizeReply {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = conn.accept_bi().await.unwrap();
        let mut tag = [0];
        recv.read_exact(&mut tag).await.unwrap();
        let _: ShareRequest = frame::read(&mut recv, frame::MAX_REQUEST).await.unwrap();
        send.write_all(&((frame::MAX_REPLY + 1) as u32).to_be_bytes())
            .await
            .unwrap();
        conn.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn reply_length_is_bounded_before_payload_is_received() {
    let router = Router::builder(endpoint().await)
        .accept(SHARE_ALPN, OversizeReply)
        .spawn();
    let phone = endpoint().await;
    let result = ShareClient::connect(
        phone.clone(),
        router.endpoint().addr(),
        token("valid-credential"),
    )
    .await;
    assert_eq!(result.unwrap_err(), ShareError::TooLarge);
    phone.close().await;
    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn dropping_last_client_closes_the_session_even_with_an_active_stream() {
    let mut setup = Setup::new(None).await;
    let client = setup.client().await;
    let stream = client.open_forward().await.unwrap();
    let (_, conn, _incoming) = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    drop(client);
    timeout(WAIT, conn.closed()).await.unwrap();
    drop(stream);
    setup.stop().await;
}

#[tokio::test]
async fn authentication_including_userinfo_has_a_total_ten_second_deadline() {
    let mut setup = Setup::new(Some(Arc::new(Notify::new()))).await;
    let conn = setup.raw().await;
    let _auth = start_request(
        &conn,
        &ShareRequest::Authenticate {
            openid: token("valid-credential"),
        },
    )
    .await;
    timeout(WAIT, setup.seen.recv()).await.unwrap().unwrap();
    timeout(Duration::from_secs(12), conn.closed())
        .await
        .unwrap();
    assert!(setup.forwards.try_recv().is_err());
    setup.stop().await;
}

#[tokio::test]
async fn friend_delete_and_reaccept_cannot_upgrade_existing_connection_epoch() {
    let mut setup = Setup::new(None).await;
    let old = setup.client().await;
    // Core increments this generation when a friend is removed and reaccepted.
    setup.handler.generation.store(8, Ordering::SeqCst);
    assert_eq!(
        old.list().await,
        Err(ShareError::Refused(ShareRefusal {
            refusal: GatewayRefusal::ShareRevoked,
            identity: None,
        }))
    );
    let mut stale_stream = old.open_forward().await.unwrap();
    let mut byte = [0];
    assert!(
        timeout(WAIT, stale_stream.read(&mut byte))
            .await
            .unwrap()
            .is_err()
    );
    assert!(setup.forwards.try_recv().is_err());
    let new = setup.client().await;
    assert_eq!(new.list().await.unwrap().user_id, USER);
    let stream = new.open_forward().await.unwrap();
    timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    drop(stream);
    old.close();
    new.close();
    setup.stop().await;
}

#[tokio::test]
async fn unknown_initial_control_is_refused_without_authentication() {
    let setup = Setup::new(None).await;
    let conn = setup.raw().await;
    assert!(matches!(
        request(&conn, &ShareRequest::Other).await,
        ShareReply::Refused { .. }
    ));
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 0);
    conn.close(0u32.into(), b"done");
    setup.stop().await;
}

#[tokio::test]
async fn trailing_bytes_are_not_accepted_as_part_of_an_authentication_frame() {
    let setup = Setup::new(None).await;
    let conn = setup.raw().await;
    let body = serde_json::to_vec(&ShareRequest::Authenticate {
        openid: token("valid-credential"),
    })
    .unwrap();
    let (mut send, _) = conn.open_bi().await.unwrap();
    send.write_all(&[frame::CONTROL]).await.unwrap();
    send.write_all(&(body.len() as u32).to_be_bytes())
        .await
        .unwrap();
    send.write_all(&body).await.unwrap();
    send.write_all(b"unexpected trailing payload")
        .await
        .unwrap();
    send.finish().unwrap();
    timeout(WAIT, conn.closed()).await.unwrap();
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 0);
    setup.stop().await;
}

#[tokio::test]
async fn active_limit_includes_streams_retained_outside_the_handler_task() {
    let mut setup = Setup::with_limits(
        None,
        ShareLimits {
            max_active_streams: 2,
        },
    )
    .await;
    let client = setup.client().await;
    let first = client.open_forward().await.unwrap();
    let retained_first = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    let second = client.open_forward().await.unwrap();
    let retained_second = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    // Server handler futures have both returned; the duplexes still hold slots.
    let result = timeout(Duration::from_millis(100), client.open_forward()).await;
    match result {
        Err(_) => {} // QUIC stream capacity itself may apply backpressure.
        Ok(Err(_)) => {}
        Ok(Ok(mut stream)) => {
            let mut byte = [0];
            assert!(
                timeout(WAIT, stream.read(&mut byte))
                    .await
                    .unwrap()
                    .is_err()
            );
        }
    }
    assert!(setup.forwards.try_recv().is_err());
    drop(retained_first);
    drop(retained_second);
    drop(first);
    drop(second);
    client.close();
    setup.stop().await;
}
