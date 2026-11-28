//! 旧手机自己没有完整的密钥：开着“添加设备”也不自动接上，新手机仍在等，不转去恢复密钥。

use wonton_client::{KeyHandoff, VerificationStage};

use super::super::support::{Home, Phone};
use super::{assert_without_keys, flow_at, invite, old_phone, pair};

/// 旧手机自己没有完整的密钥：开着“添加设备”也不自动接上，页上说明；新手机仍在等，不转去恢复密钥。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_old_phone_without_complete_keys_does_not_accept() {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let old = Phone::open(old_dir.path());
    let new = Phone::open(new_dir.path());
    let home = Home::start(&[old.device_id()]).await;
    old_phone(&home, &old, false).await;

    let code = invite(&old).await;
    pair(&new, code);
    new.signed_in().await;
    let request = flow_at(&old, "旧手机不自动接上", |flow, handoff| {
        !flow.started_here
            && flow.stage == VerificationStage::Incoming
            && handoff == Some(KeyHandoff::CannotGive)
    })
    .await;
    flow_at(&new, "新手机还在等", |flow, _| {
        flow.flow_id == request.flow_id && flow.stage == VerificationStage::Requested
    })
    .await;
    assert_without_keys(&new);
}
