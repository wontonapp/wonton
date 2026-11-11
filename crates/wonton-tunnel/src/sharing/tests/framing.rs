use super::*;
use iroh::protocol::{AcceptError, ProtocolHandler};

#[derive(Debug)]
struct ExactReply;

impl ProtocolHandler for ExactReply {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = conn.accept_bi().await.unwrap();
        let mut tag = [0];
        recv.read_exact(&mut tag).await.unwrap();
        let _: ShareRequest = frame::read(&mut recv, frame::MAX_REQUEST).await.unwrap();
        let mut body = serde_json::to_vec(&ShareReply::Shares {
            shares: shares(USER),
        })
        .unwrap();
        body.resize(frame::MAX_REPLY, b' ');
        assert_eq!(body.len(), frame::MAX_REPLY);
        send.write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        send.write_all(&body).await.unwrap();
        send.finish().unwrap();
        conn.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn client_accepts_reply_at_exact_capacity() {
    let router = Router::builder(endpoint().await)
        .accept(SHARE_ALPN, ExactReply)
        .spawn();
    let phone = endpoint().await;
    let client = ShareClient::connect(
        phone.clone(),
        router.endpoint().addr(),
        token("valid-credential"),
    )
    .await
    .unwrap();
    assert_eq!(client.user_id(), USER);
    client.close();
    phone.close().await;
    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn oversized_outbound_request_is_rejected_locally_without_token_echo() {
    let setup = Setup::new(None).await;
    let huge = "s".repeat(frame::MAX_REQUEST + 1);
    let result = ShareClient::connect(
        setup.phone.clone(),
        setup.router.endpoint().addr(),
        token(&huge),
    )
    .await
    .unwrap_err();
    assert_eq!(result, ShareError::TooLarge);
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 0);
    setup.stop().await;
}
