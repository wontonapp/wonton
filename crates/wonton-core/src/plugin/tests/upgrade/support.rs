use super::*;

pub(crate) const REPO: &str = "acme/hello";
pub(crate) const SOURCE: &str = "github:acme/hello";
pub(crate) const GIB: u64 = 1 << 30;

/// `web` 的镜像跟着版本走，`services` 追加在后面，`manifest` 追加在清单里。
pub(crate) fn hello(version: &str, services: &str, manifest: &str) -> String {
    format!(
        "services:\n  web:\n    image: web:{version}\n    volumes: ['data:/data']\n{services}volumes:\n  data: {{}}\n\
         x-wonton:\n  spec: \"0.2\"\n  id: app.hello\n  version: {version}\n  kind: app\n  title: {{zh: 你好, en: Hello}}\n  \
         description: {{zh: 打个招呼, en: Say hi}}\n  release_notes: {{zh: 版本 {version} 的说明, en: Notes}}\n{manifest}"
    )
}

pub(crate) fn release(github: &FakeGitHub, compose: &str) -> String {
    let package = PluginPackage::parse(compose, &super::langs()).unwrap();
    let tag = format!("v{}", package.manifest.version);
    github.publish(REPO, &tag, &[("docker-compose.yml", compose)])
}

pub(crate) fn id() -> PluginId {
    PluginId::parse("app.hello").unwrap()
}

pub(crate) fn web() -> String {
    container_name("app-hello", "web")
}

pub(crate) fn permissions(compose: &str) -> Vec<Permission> {
    PluginPackage::parse(compose, &super::langs())
        .unwrap()
        .manifest
        .permissions
}

pub(crate) async fn with_github(
    in_container: bool,
) -> (
    FakeGitHub,
    FakeDocker,
    Arc<PluginManager>,
    tempfile::TempDir,
) {
    let (github, client): (FakeGitHub, GitHub) = FakeGitHub::start().await;
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = if in_container {
        manager_in_container(engine, &dir)
    } else {
        manager_with(engine, &dir)
    };
    (github, docker, Arc::new(m.with_github(client)), dir)
}

pub(crate) async fn install(m: &Arc<PluginManager>, source: &str) {
    let preview = m.preview_source(source, &zh()).await.unwrap();
    let request = InstallRequest {
        scope: None,
        audience: None,
        source: preview.source.clone().unwrap(),
        digest: preview.digest.clone(),
        plan: preview.plan.clone(),
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
        remote_oauth: None,
    };
    m.install_source(&request).await.unwrap();
}

/// 主人在确认页上点了确认。
pub(crate) async fn upgrade(m: &Arc<PluginManager>) -> Result<(), InstallError> {
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
        .await?;
    let request = UpgradeRequest {
        devices: Vec::new(),
        source: None,
        digest: preview.digest,
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        lan_webui: false,
        connect_plugins: Vec::new(),
    };
    m.upgrade_source(&id(), &request, &super::langs()).await
}

pub(crate) fn version(m: &PluginManager) -> String {
    m.installed_plugin(&id(), &zh()).unwrap().version
}

pub(crate) fn image(docker: &FakeDocker, container: &str) -> String {
    docker.created(container).unwrap()["Image"]
        .as_str()
        .unwrap()
        .to_owned()
}

pub(crate) fn memory_only(memory: u64) -> wonton_protocol::PluginLimits {
    wonton_protocol::PluginLimits {
        memory: Some(memory),
        cpu_percent: None,
    }
}

/// Bot 插件，出网。
pub(crate) fn echo(version: &str, services: &str) -> String {
    format!(
        "services:\n  bot:\n    image: echo:{version}\n    environment:\n      \
         TOKEN: ${{WONTON_MATRIX_ACCESS_TOKEN}}\n      AUTH: Bearer ${{WONTON_MATRIX_ACCESS_TOKEN}}\n{services}\
         x-wonton:\n  spec: '0.4'\n  id: app.echo\n  version: {version}\n  kind: app\n  title: {{zh: 回声, en: Echo}}\n  \
         description: {{zh: 回声, en: Echo}}\n  bot: {{display_name: {{zh: 回声, en: Echo}}}}\n  \
         permissions: [matrix.bot, network.egress]\n"
    )
}

/// 新版本多了一个起不来的服务：升级被整体退回，报 rolled-back，bot 的设备和代理环境都不变。
pub(crate) async fn upgrade_expect_rollback(
    m: &Arc<PluginManager>,
    docker: &FakeDocker,
    id: &PluginId,
    bot: &str,
    token: &str,
) {
    let helper = container_name("app-echo", "helper");
    docker.set_state(
        &helper,
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    let broken = echo("1.1.0", "  helper:\n    image: helper:1\n");
    let err = m
        .upgrade_upload(
            id,
            PluginPackage::parse(&broken, &super::langs()).unwrap(),
            permissions(&broken),
            Vec::new(),
            false,
            &[],
            &Default::default(),
            &super::langs(),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(&super::message(
            "plugin-settings-upgrade-rolled-back",
            &[("new", "1.1.0"), ("version", "1.0.0")],
        )),
        "{err}"
    );
    let env = env_of_container(docker, bot);
    assert_eq!(env["TOKEN"], token, "退回旧版本也沿用同一台设备");
    assert_eq!(env["HTTP_PROXY"], "http://egress.wonton.internal:3128");
}
