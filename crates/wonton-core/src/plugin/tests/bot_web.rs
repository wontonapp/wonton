//! Bot web reservations and rendering remain bound to the complete Bot identity.

use wonton_protocol::{BotRef, bot_web_label};

use super::super::bot_services::BotVars;
use super::super::fake_docker::FakeDocker;
use super::super::render::{dry_run_context, render};
use super::super::{PluginId, PluginPackage};
use super::manager::manager_in_container;

fn package(id: &str, oidc: bool) -> PluginPackage {
    let web = if oidc {
        "  webui: {service: web, port: 80, path: /ui, sso: oidc, oidc: {redirect_paths: [/callback]}}"
    } else {
        "  webui: {service: web, port: 80, path: /ui}"
    };
    let credentials = if oidc {
        "      CLIENT: ${WONTON_OIDC_CLIENT_ID}\n      SECRET: ${WONTON_OIDC_CLIENT_SECRET}\n"
    } else {
        ""
    };
    let source = super::package(
        &format!("  web:\n    image: nginx\n    environment:\n      URL: ${{WONTON_APP_URL}}\n{credentials}  shared:\n    image: nginx\n    environment:\n      URL: ${{WONTON_APP_URL}}"),
        &format!("  agent: {{protocol: matrix}}\n  permissions: [matrix.bot]\n  bots: {{services: [web]}}\n{web}"),
    ).replace("id: app.test", &format!("id: {id}")).replace("kind: app", "kind: agent");
    PluginPackage::parse(&source, &super::langs()).unwrap()
}

#[tokio::test]
async fn collision_is_persisted_and_refused_before_provisioning_or_docker() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_container(engine.clone(), &dir);
    let name = "b".repeat(32);
    let first = package("a.xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx0000eb1d", false);
    let second = package("a.xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx00020c00", false);
    manager.reserve_bot_web(&first.manifest, &name).unwrap();
    let recreated = manager_in_container(engine, &dir);
    let error = recreated
        .provision_bot(
            &second.manifest.id,
            &second.manifest,
            &name,
            wonton_protocol::BotKeeper::Household,
            &second,
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict");
    assert!(docker.containers().is_empty());
    assert!(!dir.path().join("apps").exists());
    assert!(recreated.store.bots().all().unwrap().is_empty());
    let identity = BotRef {
        plugin_id: first.manifest.id.to_string(),
        bot: name.clone(),
    };
    assert_eq!(
        recreated
            .bot_web_identity(&bot_web_label(&identity))
            .unwrap(),
        Some(identity)
    );
    recreated
        .release_bot_web(&first.manifest.id, Some(&name))
        .unwrap();
    recreated.reserve_bot_web(&second.manifest, &name).unwrap();
}

#[tokio::test]
async fn default_web_collision_refuses_install_before_matrix_secrets_or_app_directories() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_container(engine, &dir);
    let first = package("app.foo.bar", false);
    let second = package("app.foo-bar", false);
    manager.reserve_bot_web(&first.manifest, "default").unwrap();
    let error = manager
        .install_package(second, &["matrix.bot".parse().unwrap()], super::langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict");
    assert!(docker.containers().is_empty());
    assert!(manager.store.bots().all().unwrap().is_empty());
    assert!(!dir.path().join("apps").exists());
}

#[tokio::test]
async fn two_bots_render_distinct_origins_credentials_and_callback_registrations() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_container(engine, &dir);
    let package = package("app.web-bot", true);
    reserve_default_twice(&manager, &package);
    manager
        .reserve_bot_web(&package.manifest, "family")
        .unwrap();
    let first_secret = manager
        .web_registrations()
        .unwrap()
        .values()
        .find(|entry| entry.bot.bot == "family")
        .unwrap()
        .secret
        .clone();
    manager
        .reserve_bot_web(&package.manifest, "family")
        .unwrap();
    assert_eq!(
        first_secret,
        manager
            .web_registrations()
            .unwrap()
            .values()
            .find(|entry| entry.bot.bot == "family")
            .unwrap()
            .secret
    );
    let mut ctx = dry_run_context(&package);
    ctx.bots.insert("family".to_owned(), BotVars::default());
    manager
        .fill_bot_web_vars(&package.manifest, &mut ctx)
        .unwrap();
    let plan = render(&package, &ctx, &super::langs()).unwrap();
    let env = |bot| {
        super::env_of(
            plan.containers
                .iter()
                .find(|c| c.bot.as_deref() == Some(bot))
                .unwrap(),
        )
    };
    let default = env("default");
    let family = env("family");
    assert_ne!(ctx.bots["default"].app_url, ctx.bots["family"].app_url);
    assert_ne!(ctx.bots["default"].oidc, ctx.bots["family"].oidc);
    assert!(default.contains(&format!(
        "URL={}",
        ctx.bots["default"].app_url.as_ref().unwrap()
    )));
    assert!(family.contains(&format!(
        "CLIENT={}",
        ctx.bots["family"].oidc.as_ref().unwrap().client_id
    )));
    assert!(family.contains(&format!(
        "SECRET={}",
        ctx.bots["family"].oidc.as_ref().unwrap().client_secret
    )));
    let shared = plan
        .containers
        .iter()
        .find(|c| c.service == "shared")
        .unwrap();
    assert!(super::env_of(shared).contains(&format!("URL={}", ctx.app_url.as_ref().unwrap())));
    let mut scoped = ctx.clone();
    scoped.app_url = ctx.bots["family"].app_url.clone();
    let spec = super::super::lifecycle::client_spec(&package.manifest, &scoped, "secret");
    assert_eq!(
        spec.redirect_uris,
        vec![format!("{}/callback", scoped.app_url.as_ref().unwrap())]
    );
    assert!(
        spec.post_logout_redirect_uris
            .contains(&format!("{}/ui", scoped.app_url.as_ref().unwrap()))
    );
    let aliases = manager.runtime_core_aliases(&package.manifest).unwrap();
    for name in ["default", "family"] {
        let bot = BotRef {
            plugin_id: package.manifest.id.to_string(),
            bot: name.to_owned(),
        };
        assert!(aliases.contains(&manager.cfg.public.plugin_host(&bot_web_label(&bot))));
    }
    manager
        .release_bot_web(&PluginId::parse("app.web-bot").unwrap(), Some("family"))
        .unwrap();
    assert!(
        manager
            .bot_web_oidc_identity(&ctx.bots["family"].oidc.as_ref().unwrap().client_id)
            .unwrap()
            .is_none()
    );
}

fn reserve_default_twice(manager: &super::super::PluginManager, package: &PluginPackage) {
    manager
        .reserve_bot_web(&package.manifest, "default")
        .unwrap();
    let before = manager.web_registrations().unwrap();
    manager
        .reserve_bot_web(&package.manifest, "default")
        .unwrap();
    assert_eq!(
        before["app-web-bot"].bot,
        manager.web_registrations().unwrap()["app-web-bot"].bot
    );
}
