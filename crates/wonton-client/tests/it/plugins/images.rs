//! 插件图片：同一地址只取一次，只认图片接口下的地址。

use axum::http::Method;
use wonton_client::{CoreError, PluginImage};

use super::rig::{IMAGE, Rig};

#[tokio::test]
async fn images_are_cached_and_only_come_from_the_image_api() {
    let rig = Rig::new().await;
    rig.connected().await;
    let image = PluginImage {
        path: IMAGE.to_owned(),
    };
    for _ in 0..2 {
        assert_eq!(
            rig.app.load_plugin_image(image.clone()).await.unwrap(),
            [0x89, 0x50]
        );
    }
    assert_eq!(
        rig.core.seen(&Method::GET, IMAGE).len(),
        1,
        "同一个地址内容不变"
    );

    for elsewhere in ["/_wonton/api/devices", "/_wonton/api/images/../devices"] {
        let elsewhere = PluginImage {
            path: elsewhere.to_owned(),
        };
        assert!(matches!(
            rig.app.load_plugin_image(elsewhere).await,
            Err(CoreError::InvalidInput { .. })
        ));
    }
    assert!(
        rig.core
            .seen(&Method::GET, "/_wonton/api/devices")
            .is_empty()
    );
}
