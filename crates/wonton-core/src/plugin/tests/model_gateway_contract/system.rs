use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::plugin::PluginPackage;
use crate::plugin::compose::{NetworkMode, Service, StringOrList};
use crate::plugin::egress::core_proxy::CoreProxy;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::github::GitHub;
use crate::plugin::github_source::{self, SourceRef, Wanted};
use crate::plugin::interpolate::Vars;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::PluginKind;
use crate::plugin::names::LABEL_INSTANCE;
use crate::test_docker::Names;

const LOCK: &str = include_str!("../../../../../../plugins/system.lock");
const PROXIES: [&str; 8] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];

struct Guard {
    // Containers must disappear before their bind mounts and shared data are removed.
    names: Names,
    files: tempfile::TempDir,
}

pub(super) async fn start(socket: &Path) -> super::Target {
    let package = locked_package().await;
    let spec = package
        .manifest
        .provides
        .as_ref()
        .unwrap()
        .models
        .clone()
        .unwrap();
    let mut guard = Guard {
        names: Names::new(),
        files: tempfile::tempdir().unwrap(),
    };
    let vars = fixture_vars(guard.files.path());
    let services = package
        .services(|_| &vars, &mut BTreeMap::new(), &Languages::default())
        .unwrap();
    let network = guard.names.oneshot_network("gateway");
    checked(&["network", "create", "--internal", &network]);
    let backend_url = relay(&mut guard.names, &network, socket);
    let volume = data_volume(&guard.names);
    let main = services.get(&spec.service).unwrap();
    assert!(main.network_mode.is_none());
    initialize_volume(&mut guard.names, &volume, main);
    let container = guard.names.oneshot_container("gateway");
    run_service(
        &package,
        main,
        &container,
        &network,
        &volume,
        &vars,
        guard.files.path(),
    );
    let upstream = container_address(&container, &network, spec.port);
    wait_for_gateway(&upstream, &spec.apis.openai).await;
    let probe = exec(
        &container,
        &[
            "curl".into(),
            "--connect-timeout".into(),
            "3".into(),
            "--max-time".into(),
            "5".into(),
            "--fail-with-body".into(),
            "--silent".into(),
            "--show-error".into(),
            format!("{backend_url}/models"),
        ],
        &[],
    );
    assert_eq!(probe.0, 0, "isolated upstream transport: {}", probe.2);
    for (name, service) in &services {
        if name != &spec.service {
            assert_eq!(service.network_mode, Some(NetworkMode::None));
            let container = guard.names.oneshot_container(name);
            run_service(
                &package,
                service,
                &container,
                "none",
                &volume,
                &vars,
                guard.files.path(),
            );
        }
    }
    let (docker, engine) = FakeDocker::start().await;
    docker.on_exec_output(move |name, argv, stdin| {
        assert!(name.ends_with("contract-provider"), "{name}");
        tokio::task::block_in_place(|| exec(&container, argv, stdin))
    });
    super::Target {
        spec,
        upstream,
        backend_url,
        docker,
        engine,
        _guard: Box::new(guard),
    }
}

async fn locked_package() -> PluginPackage {
    let fields = LOCK
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .find(|fields| fields.first() == Some(&"system"))
        .expect("the repository lock must declare a system gateway");
    assert_eq!(fields.len(), 5);
    let langs = Languages::default();
    let mut source = SourceRef::parse(fields[2], &langs).unwrap();
    // Fetch immutable bytes, not the plugin's live main branch or a moving release tag.
    source.wanted = Wanted::Commit(fields[3].to_owned());
    let github = GitHub::new(&CoreProxy::default());
    let origin = github_source::resolve(&github, &source, &langs)
        .await
        .unwrap();
    assert_eq!(origin.commit, fields[3]);
    let package = github_source::fetch(&github, &origin, &langs)
        .await
        .unwrap();
    assert_eq!(package.digest(), fields[4], "locked package digest changed");
    assert_eq!(package.manifest.kind, PluginKind::System);
    package
}

fn fixture_vars(root: &Path) -> Vars {
    BTreeMap::from([
        ("TZ".to_owned(), "UTC".to_owned()),
        ("WONTON_APP_DATA".to_owned(), root.display().to_string()),
        (
            "WONTON_SECRET_WEB_KEY".to_owned(),
            "contract-web-key".to_owned(),
        ),
    ])
}

fn data_volume(names: &Names) -> String {
    let volume = format!("wonton-{}-gateway-data", names.name());
    let label = format!("{LABEL_INSTANCE}={}", names.name());
    checked(&["volume", "create", "--label", &label, &volume]);
    volume
}

fn initialize_volume(names: &mut Names, volume: &str, service: &Service) {
    let container = names.oneshot_container("gateway-data");
    let image = wonton_test_images::image(&service.image);
    let user = service
        .user
        .as_ref()
        .expect("gateway data owner")
        .as_string();
    let mount = format!("type=volume,src={volume},dst=/config");
    checked(&[
        "run",
        "--name",
        &container,
        "--network",
        "none",
        "--user",
        "0:0",
        "--mount",
        &mount,
        "--entrypoint",
        "sh",
        &image,
        "-c",
        "chown \"$1\" /config",
        "sh",
        &user,
    ]);
}

fn run_service(
    package: &PluginPackage,
    service: &Service,
    container: &str,
    network: &str,
    volume: &str,
    vars: &Vars,
    root: &Path,
) {
    let mut command = docker();
    command.args(["run", "--detach", "--name", container, "--network", network]);
    // Docker's embedded DNS can otherwise forward external lookups even on an internal network.
    if network != "none" {
        command.args(["--dns", "127.0.0.1"]);
    }
    command.args(["--mount", &format!("type=volume,src={volume},dst=/config")]);
    if let Some(user) = &service.user {
        command.args(["--user", &user.as_string()]);
    }
    for proxy in PROXIES {
        command.args(["--env", &format!("{proxy}=")]);
    }
    if let Some(environment) = &service.environment {
        for (key, value) in environment
            .pairs("environment", &Languages::default())
            .unwrap()
        {
            assert!(!PROXIES.contains(&key.as_str()));
            // In particular, the declared web secret is never placed in process argv.
            command.args(["--env", &key]).env(key, value);
        }
    }
    mount_configs(&mut command, package, service, vars, root);
    let entrypoint = service.entrypoint.as_ref().map(argv);
    if let Some(entrypoint) = &entrypoint {
        assert!(!entrypoint.is_empty());
        command.args(["--entrypoint", &entrypoint[0]]);
    }
    command.arg(wonton_test_images::image(&service.image));
    if let Some(entrypoint) = &entrypoint {
        command.args(&entrypoint[1..]);
    }
    if let Some(args) = &service.command {
        command.args(argv(args));
    }
    require_success(
        command.output().unwrap(),
        "starting declared gateway service",
    );
}

fn mount_configs(
    command: &mut Command,
    package: &PluginPackage,
    service: &Service,
    vars: &Vars,
    root: &Path,
) {
    for config in &service.configs {
        let config = config.long();
        let data = package
            .config_data(
                &config.source,
                vars,
                &mut Default::default(),
                &Languages::default(),
            )
            .unwrap();
        let source = root.join(&config.source);
        std::fs::write(&source, data).unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o444)).unwrap();
        let target = config
            .target
            .unwrap_or_else(|| format!("/{}", config.source));
        command.args([
            "--mount",
            &format!("type=bind,src={},dst={target},readonly", source.display()),
        ]);
    }
}

fn argv(value: &StringOrList) -> Vec<String> {
    match value {
        StringOrList::Many(argv) => argv.clone(),
        StringOrList::One(_) => panic!("the locked services must declare explicit argv"),
    }
}

fn relay(names: &mut Names, network: &str, socket: &Path) -> String {
    // A Unix relay keeps the identical upstream handler reachable without host firewall exceptions.
    // The existing relay image runs separately; no GPL code is linked into the test binary.
    let container = names.oneshot_container("upstream");
    let directory = socket.parent().unwrap();
    let mount = format!(
        "type=bind,src={},dst=/fixture,readonly",
        directory.display()
    );
    let image = crate::plugin::host_relay::image();
    checked(&[
        "run",
        "--detach",
        "--name",
        &container,
        "--network",
        network,
        "--mount",
        &mount,
        &image,
        "TCP-LISTEN:8000,reuseaddr,fork",
        "UNIX-CONNECT:/fixture/upstream.sock",
    ]);
    let address = container_address(&container, network, 8000);
    format!("http://{address}/v1")
}

fn container_address(container: &str, network: &str, port: u16) -> String {
    let format = format!("{{{{(index .NetworkSettings.Networks \"{network}\").IPAddress}}}}");
    let output = checked(&["inspect", "--format", &format, container]);
    let ip: std::net::Ipv4Addr = output.trim().parse().unwrap();
    assert!(!ip.is_loopback());
    format!("{ip}:{port}")
}

async fn wait_for_gateway(upstream: &str, base: &str) {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(response) = client
            .get(format!("http://{upstream}{base}/models"))
            .send()
            .await
        {
            assert_eq!(
                response.status(),
                reqwest::StatusCode::UNAUTHORIZED,
                "host requests must not receive a container-loopback authentication exemption"
            );
            return;
        }
        assert!(Instant::now() < deadline, "locked gateway did not start");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn exec(container: &str, argv: &[String], stdin: &[u8]) -> (i64, String, String) {
    let mut command = docker();
    command
        .args(["exec", "--interactive", container])
        .args(argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code().map_or(-1, i64::from),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn docker() -> Command {
    let mut command = Command::new("docker");
    for proxy in PROXIES {
        command.env_remove(proxy);
    }
    command
}

fn checked(args: &[&str]) -> String {
    let output = require_success(
        docker().args(args).output().unwrap(),
        "preparing gateway fixture",
    );
    String::from_utf8(output.stdout).unwrap()
}

fn require_success(output: Output, operation: &str) -> Output {
    assert!(
        output.status.success(),
        "{operation}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
