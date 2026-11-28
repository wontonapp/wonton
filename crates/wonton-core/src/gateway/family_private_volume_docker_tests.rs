//! 真实 Docker：私人 Bot 自己的卷（compose 匿名卷、镜像的 VOLUME）在撤权收紧、核心重启、显式重试和重新授权之后
//! 还是原来的卷、原来的内容，和主人默认 Bot 的卷分开；收紧之后到重新授权之前，私人空间一直写不进去。

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;

use bollard::models::ContainerConfig;
use bollard::query_parameters::{CommitContainerOptionsBuilder, RemoveImageOptionsBuilder};
use futures_util::FutureExt;

use super::*;
use crate::plugin::names::Instance;
use crate::test_docker::Names;

const BUSYBOX: &str = "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";
const SLUG: &str = "app-echo";
/// compose 里的匿名卷和镜像自带的 VOLUME。
const TARGETS: [&str; 2] = ["/scratch", "/cache"];

fn package(image: &str) -> String {
    format!(
        r#"
services:
  bot:
    image: {image}
    init: true
    command: ["sleep", "3600"]
    environment:
      TOKEN: ${{WONTON_MATRIX_ACCESS_TOKEN}}
      ROOM: ${{WONTON_MATRIX_HOME_ROOM}}
    volumes: ["${{WONTON_BOT_DATA}}:/data", "/scratch"]
x-wonton:
  spec: '0.5'
  id: app.echo
  version: 1.0.0
  kind: agent
  title: {{zh: 回声, en: Echo}}
  description: {{zh: 回声, en: Echo}}
  agent: {{protocol: matrix}}
  bots: {{services: [bot]}}
  permissions: [matrix.bot, storage.home:write]
"#
    )
}

struct Real {
    dir: tempfile::TempDir,
    engine: Engine,
    fake: fake::Fake,
    matrix: Arc<Matrix>,
    instance: Instance,
    gateway: Arc<Gateway>,
}

impl Real {
    async fn start(engine: Engine, instance: Instance, image: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (fake, config) = fake::start().await;
        let matrix = Arc::new(
            Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
                .await
                .unwrap(),
        );
        let (tunnel, members) = loopback_tunnel(dir.path()).await;
        let gateway = open(&dir, &engine, &matrix, &instance, tunnel, members);
        let real = Self {
            dir,
            engine,
            fake,
            matrix,
            instance,
            gateway,
        };
        real.gateway
            .plugins
            .set_default_bot_plugin(Some(PluginId::parse(AGENT).unwrap()));
        let permissions = vec![
            "matrix.bot".parse().unwrap(),
            "storage.home:write".parse().unwrap(),
        ];
        real.gateway
            .plugins
            .install(&package(image), &permissions, Languages::default())
            .await
            .unwrap();
        real
    }

    /// 核心重启：新的插件管理器按记录恢复，再像启动时一样同步一次成员。
    async fn restart(&mut self) -> Result<(), crate::plugin::InstallError> {
        let members = Arc::new(Members::open(self.dir.path().join("members.json")).unwrap());
        let tunnel = self.gateway.tunnel.clone();
        self.gateway = open(
            &self.dir,
            &self.engine,
            &self.matrix,
            &self.instance,
            tunnel,
            members,
        );
        self.gateway.plugins.restore().await.unwrap();
        self.gateway.sync_members().await
    }

    fn bot(&self) -> BotRef {
        self.gateway
            .members
            .get("alice")
            .unwrap()
            .private_bot
            .unwrap()
    }

    async fn sh(&self, container: &str, script: &str) -> (i64, String) {
        let argv = ["sh", "-c", script].map(str::to_owned);
        let output = self
            .engine
            .exec_output(container, &argv, None, Duration::from_secs(20))
            .await
            .unwrap();
        (output.exit_code, output.stdout.trim().to_owned())
    }

    /// 挂载点 → Docker 实际挂的卷名。
    async fn volumes(&self, container: &str) -> BTreeMap<String, String> {
        let info = self.engine.inspect_container(container).await.unwrap();
        info.unwrap()
            .mounts
            .unwrap_or_default()
            .into_iter()
            .filter(|mount| mount.typ.as_deref() == Some("volume"))
            .filter_map(|mount| Some((mount.destination?, mount.name?)))
            .collect()
    }
}

fn open(
    dir: &tempfile::TempDir,
    engine: &Engine,
    matrix: &Arc<Matrix>,
    instance: &Instance,
    tunnel: Tunnel,
    members: Arc<Members>,
) -> Arc<Gateway> {
    let ids = fs::metadata(dir.path()).unwrap();
    Bed::gateway(
        dir.path(),
        engine.clone(),
        matrix.clone(),
        tunnel,
        members,
        crate::plugin::hostdir::Owner {
            uid: ids.uid(),
            gid: ids.gid(),
        },
        true,
        Some(instance),
        None,
    )
}

/// 两个 Bot 的卷各是各的、名字由 Bot 和挂载点定，写进去的内容在后面每一步都原样还在。
async fn assert_volumes(real: &Real, bot: &str, expected: &str) -> BTreeMap<String, String> {
    let container = real.instance.bot_container(SLUG, "bot", bot);
    let volumes = real.volumes(&container).await;
    for target in TARGETS {
        let name = real.instance.bot_volume(SLUG, "bot", bot, target);
        assert_eq!(
            volumes.get(target),
            Some(&name),
            "{bot} {target}: {volumes:?}"
        );
        let labels = real.engine.volume_labels(&name).await.unwrap().unwrap();
        assert_eq!(labels.get("wonton.bot").map(String::as_str), Some(bot));
        let (code, content) = real.sh(&container, &format!("cat {target}/who")).await;
        assert_eq!((code, content.as_str()), (0, expected), "{bot} {target}");
    }
    volumes
}

async fn home_writable(real: &Real, bot: &str, file: &str) -> bool {
    let container = real.instance.bot_container(SLUG, "bot", bot);
    real.sh(&container, &format!("touch /wonton/home/{file}"))
        .await
        .0
        == 0
}

async fn set_ai(real: &Real, ai: &str) -> Reply {
    call(
        &real.gateway,
        Method::PUT,
        &wonton_protocol::role_path("family"),
        json!({"name": "Private members", "permissions": {"private_bot": true, "ai": ai}}),
    )
    .await
}

async fn exercise(real: &mut Real) {
    let (id, _) = pending(&real.gateway, "alice", true);
    let reply = approve(&real.gateway, &id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let bot = real.bot();
    for (name, who) in [("default", "owner"), (bot.bot.as_str(), "alice")] {
        let container = real.instance.bot_container(SLUG, "bot", name);
        let script = format!("echo {who} > /scratch/who && echo {who} > /cache/who");
        assert_eq!(real.sh(&container, &script).await.0, 0);
    }
    let owner = assert_volumes(real, "default", "owner").await;
    let alice = assert_volumes(real, &bot.bot, "alice").await;
    assert!(
        owner
            .values()
            .all(|name| !alice.values().any(|other| other == name))
    );
    assert!(home_writable(real, &bot.bot, "granted").await);

    real.fake.state().refuse_profile = true;
    let reply = set_ai(real, "read_only").await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert!(!home_writable(real, &bot.bot, "restricted").await);
    assert_eq!(assert_volumes(real, &bot.bot, "alice").await, alice);

    assert!(real.restart().await.is_err());
    assert!(!home_writable(real, &bot.bot, "restarted").await);
    assert_eq!(assert_volumes(real, &bot.bot, "alice").await, alice);

    real.fake.state().refuse_profile = false;
    let reply = call(&real.gateway, Method::POST, MEMBER_SYNC_PATH, Value::Null).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert!(!home_writable(real, &bot.bot, "retried").await);
    assert_eq!(real.bot(), bot);
    assert_eq!(assert_volumes(real, &bot.bot, "alice").await, alice);

    let reply = set_ai(real, "self_confirms").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(home_writable(real, &bot.bot, "regranted").await);
    assert_eq!(assert_volumes(real, &bot.bot, "alice").await, alice);
    assert_eq!(assert_volumes(real, "default", "owner").await, owner);
    println!(
        "{}: owner {owner:?} alice {alice:?}；收紧 503 → 重启仍失败 → 重试 204 都不可写，卷和内容不变；重新授权可写",
        real.instance.name().unwrap_or_default()
    );
}

/// 一份自带 `VOLUME /cache` 的镜像：从 busybox 容器提交，标签归这次测试。
async fn image_with_volume(engine: &Engine, names: &mut Names) -> String {
    let base = crate::test_docker::ensure_image(engine, &wonton_test_images::image(BUSYBOX)).await;
    let seed = names.oneshot_container("bot-volume-seed");
    engine
        .create_container(
            &seed,
            bollard::models::ContainerCreateBody {
                image: Some(base),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let repo = names.oneshot_image("bot-volume");
    engine
        .docker
        .commit_container(
            CommitContainerOptionsBuilder::default()
                .container(&seed)
                .repo(&repo)
                .tag("v1")
                .pause(false)
                .build(),
            ContainerConfig {
                volumes: Some(vec!["/cache".to_owned()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.remove_container(&seed).await.unwrap();
    format!("{repo}:v1")
}

#[tokio::test]
#[ignore = "需要 Docker，验证私人 Bot 的匿名卷和镜像卷在撤权、核心重启、重试后不丢、不可写"]
async fn private_bot_volumes_survive_restriction_restart_and_retry() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let mut names = Names::new();
    let image = image_with_volume(&engine, &mut names).await;
    // 安装和成员同步的 future 很大，放到堆上，不然测试线程的栈不够。
    let outcome = AssertUnwindSafe(Box::pin(async {
        let mut real = Box::pin(Real::start(
            engine.clone(),
            names.instance().clone(),
            &image,
        ))
        .await;
        Box::pin(exercise(&mut real)).await;
    }))
    .catch_unwind()
    .await;
    drop(names);
    let removed = engine
        .docker
        .remove_image(
            &image,
            Some(RemoveImageOptionsBuilder::default().force(true).build()),
            None,
        )
        .await;
    assert!(removed.is_ok(), "Docker cleanup failed: {removed:?}");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
