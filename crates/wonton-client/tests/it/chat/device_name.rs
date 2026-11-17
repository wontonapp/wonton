//! 配对时填的设备名要成为这台设备在 Matrix 上的显示名，确认弹窗里才看得到是谁在请求。

use std::str::FromStr;

use wonton_client::{Action, SecretString};
use wonton_matrix::OwnedUserId;

use super::support::{Home, OWNER, Phone};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_name_typed_while_pairing_becomes_the_matrix_device_name() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::PairHome {
                code: SecretString::new(home.pairing_code()),
                device_name: "fileapp-iphone".to_owned(),
            },
        })
        .unwrap();
    let (_, device_id) = phone.signed_in().await;

    let admin = home.matrix.admin().await;
    let owner = OwnedUserId::from_str(OWNER).unwrap();
    let devices = admin.devices(&owner).await;
    assert!(
        devices
            .iter()
            .any(|(id, name)| id == &device_id && name.as_deref() == Some("fileapp-iphone")),
        "设备列表里没有 {device_id} 和它的名字：{devices:?}"
    );
}
