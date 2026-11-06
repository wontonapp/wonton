//! Bot 资料读写的可见结果、临时设备回收，以及媒体下载边界。

use std::collections::BTreeMap;

use hyper::Method;
use serde_json::json;
use wonton_protocol::{AVATARS_FIELD, AvatarMedia, MAX_AVATAR_BYTES, StateAvatars};

use super::fake::{Fake, start};
use super::{Matrix, MatrixError, Session};
use crate::http::HttpClient;

async fn setup() -> (Fake, Matrix, tempfile::TempDir) {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    matrix.login("echo", "插件").await.unwrap();
    (fake, matrix, dir)
}

pub(super) fn sample_avatars() -> StateAvatars {
    StateAvatars {
        states: BTreeMap::from([
            (
                "idle".into(),
                AvatarMedia {
                    url: "mxc://wonton.localhost/idle".into(),
                    mimetype: "video/mp4".into(),
                    width: 32,
                    height: 32,
                    size: 123,
                    duration_ms: Some(1000),
                    poster: Some("mxc://wonton.localhost/poster".into()),
                },
            ),
            (
                "future".into(),
                AvatarMedia {
                    url: "mxc://wonton.localhost/future".into(),
                    mimetype: "image/png".into(),
                    width: 32,
                    height: 32,
                    size: 42,
                    duration_ms: None,
                    poster: None,
                },
            ),
        ]),
    }
}

#[tokio::test]
async fn bot_profile_round_trips_and_absence_differs_from_an_empty_field() {
    let (fake, matrix, _dir) = setup().await;
    let before = matrix.devices("@echo:test").await.unwrap();
    let avatars = sample_avatars();
    matrix
        .as_bot("@echo:test", async |bot: &Session<'_>| {
            assert_eq!(bot.avatar_url().await?, None);
            assert_eq!(bot.avatars().await?, None);
            bot.set_display_name("小云").await?;
            bot.set_avatar_url(Some("mxc://test/static")).await?;
            bot.set_avatars(&avatars).await?;
            assert_eq!(bot.display_name().await?, Some("小云".into()));
            assert_eq!(bot.avatar_url().await?, Some("mxc://test/static".into()));
            assert_eq!(bot.avatars().await?, Some(avatars.clone()));
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(matrix.devices("@echo:test").await.unwrap(), before);
    assert_eq!(
        fake.state().user("@echo:test").avatars,
        Some(json!(avatars))
    );
    matrix
        .as_bot("@echo:test", async |bot: &Session<'_>| {
            bot.set_avatar_url(None).await?;
            bot.set_avatars(&StateAvatars::default()).await?;
            assert_eq!(bot.avatar_url().await?, None);
            assert_eq!(bot.avatars().await?, Some(StateAvatars::default()));
            bot.delete_avatars().await?;
            assert_eq!(bot.avatars().await?, None);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(fake.state().user("@echo:test").avatar_url, None);
    assert_eq!(fake.state().user("@echo:test").avatars, None);
}

#[tokio::test]
async fn temporary_bot_devices_are_logged_out_on_closure_and_profile_errors() {
    let (fake, matrix, _dir) = setup().await;
    let before = matrix.devices("@echo:test").await.unwrap();
    let result: Result<(), _> = matrix
        .as_bot("@echo:test", async |bot: &Session<'_>| {
            bot.set_display_name("保留成功的写入").await?;
            Err(MatrixError::Invalid("closure failed".into()))
        })
        .await;
    assert!(matches!(result, Err(MatrixError::Invalid(_))));
    assert_eq!(
        fake.state().user("@echo:test").display_name.as_deref(),
        Some("保留成功的写入")
    );
    assert_eq!(matrix.devices("@echo:test").await.unwrap(), before);
    fake.state().refuse_profile = true;
    let result = matrix
        .as_bot("@echo:test", async |bot: &Session<'_>| {
            bot.set_avatars(&StateAvatars::default()).await
        })
        .await;
    assert_eq!(result.unwrap_err().errcode(), "M_UNKNOWN");
    assert_eq!(matrix.devices("@echo:test").await.unwrap(), before);
}

#[tokio::test]
async fn as_bot_refuses_unknown_remote_and_malformed_ids_without_creating_accounts() {
    let (fake, matrix, _dir) = setup().await;
    let users = fake.state().users.len();
    let tokens = fake.state().tokens.len();
    for user in [
        "@missing:test",
        "@echo:elsewhere",
        "echo:test",
        "@:test",
        "@echo:",
    ] {
        let result = matrix.as_bot(user, async |_: &Session<'_>| Ok(())).await;
        assert!(matches!(result, Err(MatrixError::Invalid(_))), "{user}");
    }
    assert_eq!(fake.state().users.len(), users);
    assert_eq!(fake.state().tokens.len(), tokens);
}

#[tokio::test]
async fn malformed_stored_state_is_not_replaced_with_empty_avatars() {
    let (fake, matrix, _dir) = setup().await;
    for bad in [
        json!(null),
        json!({}),
        json!({"states": []}),
        json!({"states": {"idle": {"url": 1}}}),
    ] {
        fake.state().users.get_mut("@echo:test").unwrap().avatars = Some(bad.clone());
        let result = matrix
            .as_bot("@echo:test", async |bot: &Session<'_>| bot.avatars().await)
            .await;
        assert!(matches!(result, Err(MatrixError::Invalid(_))), "{bad}");
        assert_eq!(fake.state().user("@echo:test").avatars, Some(bad));
        assert_eq!(matrix.devices("@echo:test").await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn profile_writes_require_the_accounts_own_token() {
    let (_fake, matrix, _dir) = setup().await;
    let owner = matrix.login("owner", "主人").await.unwrap();
    let path = format!(
        "/_matrix/client/v3/profile/{}/{}",
        super::api::encode("@echo:test"),
        AVATARS_FIELD
    );
    for method in [Method::PUT, Method::DELETE] {
        let result = matrix
            .api
            .json(
                method,
                &path,
                Some(&owner.access_token),
                Some(&json!({AVATARS_FIELD: {"states": {}}})),
                "测试越权",
            )
            .await;
        assert_eq!(result.unwrap_err().errcode(), "M_FORBIDDEN");
    }
}

#[tokio::test]
async fn media_is_uploaded_by_the_bot_and_downloaded_by_the_core_with_a_size_bound() {
    let (fake, matrix, _dir) = setup().await;
    let url = matrix
        .as_bot("@echo:test", async |bot: &Session<'_>| {
            bot.upload_media("image/png", "头像.png", b"image".to_vec())
                .await
        })
        .await
        .unwrap();
    assert_eq!(matrix.download_media(&url).await.unwrap(), b"image");
    let bytes = vec![42; MAX_AVATAR_BYTES as usize + 10_000];
    let large = matrix
        .as_bot("@echo:test", async |bot: &Session<'_>| {
            bot.upload_media("image/png", "large.png", bytes).await
        })
        .await
        .unwrap();
    let downloaded = matrix.download_media(&large).await.unwrap();
    assert_eq!(downloaded.len(), MAX_AVATAR_BYTES as usize + 1);
    assert!(downloaded.iter().all(|byte| *byte == 42));
    assert_eq!(fake.state().media_uploaders, ["@echo:test", "@echo:test"]);
    assert_eq!(
        fake.state().media_downloaders,
        ["@wonton-core:test", "@wonton-core:test"]
    );
    let error = matrix
        .download_media("mxc://test/missing")
        .await
        .unwrap_err();
    assert_eq!(error.errcode(), "M_NOT_FOUND");
}

#[tokio::test]
async fn invalid_media_addresses_never_make_a_download_request() {
    let (fake, matrix, _dir) = setup().await;
    for url in [
        "https://test/id",
        "mxc://test/",
        "mxc:///id",
        "mxc://test/id/extra",
        "mxc://test/id?x",
        "mxc://test/id#x",
        "mxc://test/%2f",
        "mxc://user@test/id",
        "mxc://test/a b",
    ] {
        assert!(
            matches!(
                matrix.download_media(url).await,
                Err(MatrixError::Invalid(_))
            ),
            "{url}"
        );
    }
    assert!(fake.state().media_downloaders.is_empty());
}

#[tokio::test]
async fn authenticated_media_download_refuses_missing_or_revoked_tokens() {
    let (_fake, matrix, _dir) = setup().await;
    let login = matrix.login("echo", "媒体上传").await.unwrap();
    let url = matrix
        .session(&login)
        .upload_media("image/png", "avatar.png", b"image".to_vec())
        .await
        .unwrap();
    let id = url.rsplit_once('/').unwrap().1;
    let path = format!("/_matrix/client/v1/media/download/test/{id}");
    matrix.session(&login).logout().await.unwrap();
    for token in [None, Some(login.access_token.as_str())] {
        let result = matrix
            .api
            .json(Method::GET, &path, token, None, "测试媒体认证")
            .await;
        assert_eq!(result.unwrap_err().errcode(), "M_UNKNOWN_TOKEN");
    }
    assert_eq!(matrix.download_media(&url).await.unwrap(), b"image");
}

#[test]
fn profile_optional_strings_do_not_silently_accept_bad_types() {
    for value in [
        json!(null),
        json!([]),
        json!({"displayname": 12}),
        json!({"displayname": {}}),
    ] {
        assert!(super::bot_profile::profile_string(&value, "displayname").is_err());
    }
    for value in [json!({}), json!({"displayname": null})] {
        assert_eq!(
            super::bot_profile::profile_string(&value, "displayname").unwrap(),
            None
        );
    }
}
