use std::os::unix::fs::MetadataExt;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt;

use super::super::docker::Engine;
use super::super::hostdir::Owner;
use super::super::package::PluginPackage;
use super::super::private_homes::HomeMount;
use super::super::render::{dry_run_context, render};

#[tokio::test]
#[ignore = "需要 Docker，验证私人空间真实挂载、0700、只读和跨人隔离"]
async fn private_home_mount_is_read_only_and_contains_only_its_keeper_files() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let image = wonton_test_images::image(
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    );
    crate::test_docker::ensure_image(&engine, &image).await;
    let dir = tempfile::tempdir().unwrap();
    let metadata = std::fs::metadata(dir.path()).unwrap();
    let owner = Owner {
        uid: metadata.uid(),
        gid: metadata.gid(),
    };
    let alice = crate::private_home::ensure(dir.path(), "alice", owner).unwrap();
    let bob = crate::private_home::ensure(dir.path(), "bob", owner).unwrap();
    std::fs::write(alice.join("alice-only"), b"alice").unwrap();
    std::fs::write(bob.join("bob-only"), b"bob").unwrap();
    assert_eq!(std::fs::metadata(&alice).unwrap().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(&bob).unwrap().mode() & 0o777, 0o700);
    let text = super::private_home::compose(&image);
    let package = PluginPackage::parse(&text, &super::langs()).unwrap();
    let mut ctx = dry_run_context(&package);
    ctx.puid = owner.uid;
    ctx.pgid = owner.gid;
    ctx.app_data = dir.path().join("apps/app-test");
    ctx.homes.insert(
        "default".into(),
        HomeMount {
            path: alice.clone(),
            read_only: true,
        },
    );
    let mut plan = render(&package, &ctx, &super::langs()).unwrap();
    let index = plan
        .containers
        .iter()
        .position(|container| container.bot.is_some())
        .unwrap();
    let mut bot = plan.containers.remove(index);
    let host = bot.body.host_config.as_mut().unwrap();
    host.network_mode = Some("none".into());
    bot.body.networking_config = None;
    for mount in host.mounts.as_ref().unwrap() {
        if mount.target.as_deref() == Some("/data") {
            std::fs::create_dir_all(mount.source.as_ref().unwrap()).unwrap();
        }
    }
    let name = format!("fr-pr13-home-{}", crate::secret::random_token());
    let outcome = AssertUnwindSafe(async {
        engine.create_container(&name, bot.body).await.unwrap();
        engine.start_container(&name).await.unwrap();
        let command = ["sh", "-c", "test \"$(cat /wonton/home/alice-only)\" = alice && test ! -e /wonton/home/bob-only && test ! -e /wonton/home/../bob && test \"$(stat -c %a /wonton/home)\" = 700 && ! touch /wonton/home/write-test"];
        let output = engine.exec_output(&name, &command.map(str::to_owned), None, Duration::from_secs(10)).await.unwrap();
        assert_eq!(output.exit_code, 0, "{}", output.stderr);
        assert!(!alice.join("write-test").exists());
    }).catch_unwind().await;
    let cleanup = engine.remove_container(&name).await;
    assert!(cleanup.is_ok(), "Docker cleanup failed: {cleanup:?}");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
