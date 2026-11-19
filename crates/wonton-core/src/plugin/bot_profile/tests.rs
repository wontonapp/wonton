use super::*;
use crate::http::HttpClient;
use crate::matrix::fake::{Fake, start};
use crate::matrix::{BotAccount, Matrix};
use crate::plugin::bots::{BotEntry, PluginBots};
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginPackage, PluginStatus};
use wonton_protocol::BotKeeper;

pub(crate) struct Bed {
    pub manager: Arc<PluginManager>,
    pub matrix: Arc<Matrix>,
    pub fake: Fake,
    pub owner: Arc<crate::owner::OwnerStore>,
    pub dir: tempfile::TempDir,
    pub id: PluginId,
}

impl Bed {
    pub async fn start() -> Self {
        let (fake, config) = start().await;
        let dir = tempfile::tempdir().unwrap();
        let http = HttpClient::new();
        let matrix = Arc::new(
            Matrix::connect(&config, dir.path(), http.clone())
                .await
                .unwrap(),
        );
        let owner = crate::owner::OwnerStore::for_test(
            dir.path(),
            &crate::owner::Owner {
                username: "owner".into(),
                display_name: "主人".into(),
                email: None,
            },
        );
        let cfg = configuration(&dir, owner.clone());
        let manager = Arc::new(
            PluginManager::new(
                Engine::unreachable_for_test(),
                Store::open(dir.path().join("plugins")).unwrap(),
                http,
                None,
                cfg,
                ImageWorker::in_process(),
            )
            .with_matrix(Some(matrix.clone())),
        );
        let id = PluginId::parse("app.echo").unwrap();
        manager.register_for_test(PluginPackage::manifest_of(
            "services:\n  bot:\n    image: echo\nx-wonton:\n  spec: '0.5'\n  id: app.echo\n  version: 1.0.0\n  kind: app\n  title: {zh: 回声, en: Echo}\n  description: {zh: 回声, en: Echo}\n  bot: {}\n  permissions: [matrix.bot]\n"
        , &Languages::from_header(Some("zh-Hans"))).unwrap(), PluginStatus::Running, None);
        let login = matrix.login("echo", "Bot").await.unwrap();
        matrix
            .session(&login)
            .set_display_name("Echo")
            .await
            .unwrap();
        manager
            .store
            .bots()
            .save(
                &id,
                &PluginBots {
                    bots: [(
                        crate::plugin::bot_services::DEFAULT_BOT.to_owned(),
                        BotEntry {
                            account: BotAccount {
                                user_id: login.user_id,
                                home_room: "!room:test".into(),
                            },
                            device_id: Some(login.device_id),
                            keeper: BotKeeper::User {
                                username: "owner".to_owned(),
                            },
                            bus_token: "bus-token".to_owned(),
                        },
                    )]
                    .into(),
                },
            )
            .unwrap();
        Self {
            manager,
            matrix,
            fake,
            owner,
            dir,
            id,
        }
    }

    pub async fn media(&self, bytes: &[u8]) -> String {
        self.matrix
            .core_session()
            .upload_media("application/octet-stream", "unknown", bytes.to_vec())
            .await
            .unwrap()
    }
}

fn configuration(
    dir: &tempfile::TempDir,
    profile: Arc<crate::owner::OwnerStore>,
) -> LifecycleConfig {
    LifecycleConfig {
        public: crate::config::PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.localhost".into(),
            port: 8080,
        },
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        relay_dir: dir.path().join("relay"),
        core_container: None,
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_timeout: std::time::Duration::from_secs(5),
        owner: "owner".into(),
        profile,
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    }
}

#[tokio::test]
async fn failed_agent_edits_do_not_use_quota_and_owner_edits_are_unlimited() {
    let bed = Bed::start().await;
    for _ in 0..12 {
        let rejected = bed
            .manager
            .set_bot_profile(
                &bed.id,
                None,
                LocalUpdate {
                    name: Some("WONTON".into()),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(
            rejected,
            Err(ProfileError::Name(BotNameProblem::Reserved))
        ));
    }
    for n in 0..10 {
        let profile = bed
            .manager
            .set_bot_profile(
                &bed.id,
                None,
                LocalUpdate {
                    name: Some(format!("Bot {n}")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(profile.name, Some(format!("Bot {n}")));
    }
    assert!(matches!(
        bed.manager
            .set_bot_profile(
                &bed.id,
                None,
                LocalUpdate {
                    name: Some("Agent blocked".into()),
                    ..Default::default()
                }
            )
            .await,
        Err(ProfileError::RateLimited {
            retry_after_secs: 1..=3600
        })
    ));
    for n in 0..12 {
        let result = bed
            .manager
            .update_bot_profile(
                &bed.id,
                BotProfileUpdate {
                    name: Some(format!("Owner {n}")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.profile_rev, 11 + n);
    }
    assert_eq!(
        bed.fake.state().user("@echo:test").display_name.as_deref(),
        Some("Owner 11")
    );
}

#[tokio::test]
async fn invalid_media_is_checked_before_any_profile_write_or_bot_upload() {
    let bed = Bed::start().await;
    let before = bed.fake.state().media.len();
    let rejected = bed
        .manager
        .set_bot_profile(
            &bed.id,
            None,
            LocalUpdate {
                name: Some("Not applied".into()),
                avatar: Some(MediaChange::Set(UploadedAvatar {
                    filename: "a.png".into(),
                    data: b"not media".to_vec(),
                    poster: None,
                })),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(
        rejected,
        Err(ProfileError::Media(AvatarMediaProblem::Format))
    ));
    assert_eq!(bed.fake.state().media.len(), before);
    assert_eq!(
        bed.fake.state().user("@echo:test").display_name.as_deref(),
        Some("Echo")
    );
    assert_eq!(bed.manager.profile_revisions().get(bed.id.as_str()), 0);
}

#[tokio::test]
async fn malformed_existing_state_prevents_mutation_before_upload_or_quota() {
    let bed = Bed::start().await;
    bed.fake
        .state()
        .users
        .get_mut("@echo:test")
        .unwrap()
        .avatars = Some(serde_json::json!({"states":[]}));
    let error = bed
        .manager
        .set_bot_profile(
            &bed.id,
            None,
            LocalUpdate {
                name: Some("Must not change".into()),
                ..Default::default()
            },
        )
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ProfileError::MatrixUnavailable(_)));
    assert_eq!(
        bed.fake.state().user("@echo:test").display_name.as_deref(),
        Some("Echo")
    );
    assert_eq!(bed.manager.profile_revisions().get("app.echo"), 0);
}

#[tokio::test]
async fn confirmed_partial_edits_and_logout_errors_still_advance_revision_and_quota() {
    let bed = Bed::start().await;
    bed.fake.state().refuse_avatar_write = true;
    for n in 0..10 {
        let error = bed
            .manager
            .set_bot_profile(
                &bed.id,
                None,
                LocalUpdate {
                    name: Some(format!("Confirmed {n}")),
                    avatar: Some(MediaChange::Clear),
                    ..Default::default()
                },
            )
            .await
            .err()
            .unwrap();
        assert!(matches!(error, ProfileError::MatrixUnavailable(_)));
        assert_eq!(
            bed.fake.state().user("@echo:test").display_name,
            Some(format!("Confirmed {n}"))
        );
        assert_eq!(bed.manager.profile_revisions().get("app.echo"), n + 1);
    }
    assert!(matches!(
        bed.manager
            .set_bot_profile(
                &bed.id,
                None,
                LocalUpdate {
                    name: Some("Blocked".into()),
                    ..Default::default()
                }
            )
            .await,
        Err(ProfileError::RateLimited { .. })
    ));
    bed.fake.state().refuse_avatar_write = false;
    bed.fake.state().refuse_logout = true;
    let error = bed
        .manager
        .update_bot_profile(
            &bed.id,
            BotProfileUpdate {
                name: Some("Owner committed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ProfileError::MatrixUnavailable(_)));
    assert_eq!(
        bed.fake.state().user("@echo:test").display_name.as_deref(),
        Some("Owner committed")
    );
    assert_eq!(bed.manager.profile_revisions().get("app.echo"), 11);
}

fn keeper() -> BotKeeper {
    BotKeeper::User {
        username: "owner".to_owned(),
    }
}

fn named(name: &str) -> LocalUpdate {
    LocalUpdate {
        name: Some(name.to_owned()),
        ..Default::default()
    }
}

/// 插件里装第二个 Bot（假 Matrix 用户 + 记录）；返回它的用户 ID。
async fn bed_with_second_bot() -> (Bed, String) {
    let bed = Bed::start().await;
    let second = bed.matrix.login("echo2", "Bot").await.unwrap();
    bed.matrix
        .session(&second)
        .set_display_name("Second")
        .await
        .unwrap();
    bed.manager.bot_records_for_test(
        &bed.id,
        &[
            (
                crate::plugin::bot_services::DEFAULT_BOT,
                "@echo:test",
                keeper(),
                "bus-token",
            ),
            ("second", &second.user_id, keeper(), "second-token"),
        ],
    );
    (bed, second.user_id)
}

fn name_of(profile: &Profile) -> Option<&str> {
    profile.name.as_deref()
}

#[tokio::test]
async fn each_bot_reads_and_writes_its_own_profile() {
    let (bed, second) = bed_with_second_bot().await;
    let profile = bed
        .manager
        .set_bot_profile(&bed.id, Some("second"), named("第二个"))
        .await
        .unwrap();
    assert_eq!(name_of(&profile), Some("第二个"));
    assert_eq!(
        bed.fake.state().user(&second).display_name.as_deref(),
        Some("第二个")
    );
    assert_eq!(
        bed.fake.state().user("@echo:test").display_name.as_deref(),
        Some("Echo")
    );
    for (bot, expected) in [
        ("second", Some("第二个")),
        (crate::plugin::bot_services::DEFAULT_BOT, Some("Echo")),
    ] {
        let profile = bed.manager.bot_profile(&bed.id, Some(bot)).await.unwrap();
        assert_eq!(name_of(&profile), expected, "{bot}");
    }
}

#[tokio::test]
async fn the_hourly_quota_is_counted_per_bot() {
    let (bed, _second) = bed_with_second_bot().await;
    for n in 0..10 {
        bed.manager
            .set_bot_profile(&bed.id, Some("second"), named(&format!("Two {n}")))
            .await
            .unwrap();
    }
    assert!(matches!(
        bed.manager
            .set_bot_profile(&bed.id, Some("second"), named("Blocked"))
            .await,
        Err(ProfileError::RateLimited { .. })
    ));
    let profile = bed
        .manager
        .set_bot_profile(
            &bed.id,
            Some(crate::plugin::bot_services::DEFAULT_BOT),
            named("Echo"),
        )
        .await
        .unwrap();
    assert_eq!(name_of(&profile), Some("Echo"));
}

#[tokio::test]
async fn unknown_bots_do_not_fall_back_to_the_default_one() {
    let (bed, second) = bed_with_second_bot().await;
    let Err(ProfileError::UnknownBot(bot)) = bed.manager.bot_profile(&bed.id, Some("ghost")).await
    else {
        panic!("读未知 Bot 应报 UnknownBot");
    };
    assert_eq!(bot, "ghost");
    let Err(error) = bed
        .manager
        .set_bot_profile(&bed.id, Some("ghost"), named("ghost"))
        .await
    else {
        panic!("写未知 Bot 应报 UnknownBot");
    };
    assert!(matches!(error, ProfileError::UnknownBot(bot) if bot == "ghost"));
    assert_eq!(
        bed.fake.state().user("@echo:test").display_name.as_deref(),
        Some("Echo")
    );

    // 记录里没有默认 Bot 时，点名的接口报错，主人的资料页仍取记录里的第一个。
    bed.manager
        .bot_records_for_test(&bed.id, &[("second", &second, keeper(), "second-token")]);
    assert!(matches!(
        bed.manager
            .bot_profile(&bed.id, Some(crate::plugin::bot_services::DEFAULT_BOT))
            .await,
        Err(ProfileError::UnknownBot(_))
    ));
    let profile = bed.manager.bot_profile(&bed.id, None).await.unwrap();
    assert_eq!(name_of(&profile), Some("Second"));
}
