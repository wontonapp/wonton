use std::fs;
use std::os::unix::fs::MetadataExt;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};
use std::sync::Arc;
use wonton_core::device::Devices;
use wonton_core::http::{HttpClient, RequestBody};
use wonton_core::push::{Push, Subscription};

mod runtime_image;
#[path = "../../src/test_docker.rs"]
mod test_docker;

const CORE: &str = env!("CARGO_BIN_EXE_wonton-core");
const BASE: &str = "public.ecr.aws/docker/library/debian:trixie-slim";

/// 基础镜像：CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images），本地用原名。
fn base() -> String {
    wonton_test_images::image(BASE)
}
const WAIT: Duration = Duration::from_secs(60);

pub struct Account {
    token: String,
    pub user_id: String,
}

pub struct Install {
    core: String,
    matrix: String,
    address: String,
    pub pushkey: String,
    _names: test_docker::Names,
    image: runtime_image::Image,
    dir: tempfile::TempDir,
}

impl Install {
    pub fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        wonton_core::install::init(dir.path(), None).unwrap();
        let mut names = test_docker::Names::new();
        let network = names.oneshot_network("push-core");
        let core = names.oneshot_container("core");
        let matrix = names.oneshot_container("tuwunel");
        let image = runtime_image::Image::build(names.oneshot_image("core"));
        let mut install = Self {
            core,
            matrix,
            address: String::new(),
            dir,
            image,
            pushkey: String::new(),
            _names: names,
        };
        let config = install.compose();
        install.seed_push();
        docker(&[
            "network",
            "create",
            "--label",
            &format!("com.docker.compose.project={}", install.core),
            "--label",
            "com.docker.compose.network=core",
            &network,
        ]);
        install.start_matrix(&config["services"]["tuwunel"], &network);
        install.start_core(&config["services"]["core"], &network);
        install.address = matrix_url(&install.matrix, &network);
        let deadline = Instant::now() + WAIT;
        loop {
            let output = Command::new("docker")
                .args(["exec", &install.core, "wonton-core", "healthcheck"])
                .output()
                .unwrap();
            if output.status.success() {
                break;
            }
            assert!(Instant::now() < deadline, "核心未就绪：{}", install.logs());
            std::thread::sleep(Duration::from_millis(250));
        }
        install
    }

    fn seed_push(&mut self) {
        let state = self.dir.path().join("data/core");
        fs::create_dir_all(&state).unwrap();
        let devices = Arc::new(Devices::open(state.join("devices.json")).unwrap());
        let device = SecretKey::generate().public();
        devices.add(device, None).unwrap();
        let server = wonton_core::tunnel::load_or_create_key(&state.join("tunnel-key")).unwrap();
        let push = Push::open(state.join("push.json"), devices, server.public()).unwrap();
        // 真实的 HTTP 端点回 404 表示订阅失效。只有带事件的通知才会发 Web Push；计数通知不能让它被删除。
        let subscription = Subscription::new(
            "http://127.0.0.1:8080/push-install-expired",
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
            "BTBZMqHH6r4Tts7J_aSIgg",
        ).unwrap();
        self.pushkey = push.register(device, subscription).unwrap().pushkey;
    }

    fn compose(&self) -> Value {
        let data = self.dir.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let uid = fs::metadata(&data).unwrap().uid();
        let gid = fs::metadata(&data).unwrap().gid();
        let env = format!(
            "WONTON_CORE_IMAGE={}\nWONTON_INSTALL_DIR={}\nWONTON_DATA_DIR={}\n\
            WONTON_BASE_DOMAIN=wonton.localhost\nWONTON_PORT=8080\nWONTON_TUNNEL_PORT=8482\n\
            WONTON_PUID={uid}\nWONTON_PGID={gid}\n",
            base(),
            self.dir.path().display(),
            data.display()
        );
        fs::write(self.dir.path().join(".env"), env).unwrap();
        let output = Command::new("docker")
            .args(["compose", "config", "--format", "json"])
            .current_dir(self.dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn start_matrix(&self, service: &Value, network: &str) {
        let mut args = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            self.matrix.clone(),
            "--network".into(),
            network.into(),
            "--label".into(),
            format!("com.docker.compose.project={}", self.core),
            "--label".into(),
            "com.docker.compose.service=tuwunel".into(),
        ];
        append_aliases(&mut args, service);
        append_environment(&mut args, service, &[]);
        append_mounts(&mut args, service);
        args.push(
            service["image"]
                .as_str()
                .unwrap()
                .split('@')
                .next()
                .unwrap()
                .into(),
        );
        args.extend(
            service["command"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned()),
        );
        docker(&args);
    }

    fn start_core(&self, service: &Value, network: &str) {
        let mut args = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            self.core.clone(),
            "--network".into(),
            network.into(),
            "--sysctl".into(),
            "net.ipv4.ip_forward=0".into(),
            "--label".into(),
            format!("com.docker.compose.project={}", self.core),
            "--user".into(),
            format!(
                "{}:{}",
                fs::metadata(self.dir.path()).unwrap().uid(),
                fs::metadata(self.dir.path()).unwrap().gid()
            ),
            "--group-add".into(),
            fs::metadata("/var/run/docker.sock")
                .unwrap()
                .gid()
                .to_string(),
        ];
        append_aliases(&mut args, service);
        append_environment(
            &mut args,
            service,
            &[
                "WONTON_CORE_CONTAINER",
                "WONTON_INSTANCE",
                "WONTON_SYSTEM_PLUGINS",
                "WONTON_DEFAULT_PLUGINS",
                "WONTON_OIDC_ADMIN_URL",
                "WONTON_OIDC_PUBLIC_URL",
            ],
        );
        args.extend(["-e".into(), format!("WONTON_CORE_CONTAINER={}", self.core)]);
        args.extend([
            "-e".into(),
            format!("WONTON_INSTANCE={}", self._names.name()),
        ]);
        append_mounts(&mut args, service);
        args.push(self.image.0.clone());
        docker(&args);
    }

    pub async fn login(&self, name: &str) -> Account {
        let key = fs::read_to_string(self.dir.path().join("secrets/matrix-jwt.pem")).unwrap();
        let issued = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = jsonwebtoken::encode(
            &Header::new(Algorithm::EdDSA),
            &json!({"sub": name, "iss": "wonton-core", "aud": "tuwunel", "iat": issued, "exp": issued + 120}),
            &EncodingKey::from_ed_pem(key.as_bytes()).unwrap(),
        ).unwrap();
        let value = self
            .request(
                Method::POST,
                "login",
                None,
                json!({
                    "type": "org.matrix.login.jwt", "token": token, "device_id": "INSTALL_PUSH",
                }),
            )
            .await;
        Account {
            token: value["access_token"].as_str().unwrap().into(),
            user_id: value["user_id"].as_str().unwrap().into(),
        }
    }

    pub async fn matrix(
        &self,
        method: Method,
        path: &str,
        account: &Account,
        body: Value,
    ) -> Value {
        self.request(method, path, Some(&account.token), body).await
    }

    pub async fn pushers(&self, account: &Account) -> Value {
        self.matrix(Method::GET, "pushers", account, Value::Null)
            .await
    }

    async fn request(&self, method: Method, path: &str, token: Option<&str>, body: Value) -> Value {
        let (status, value) = HttpClient::new()
            .request_json(
                method,
                &format!("{}/_matrix/client/v3/{path}", self.address),
                token,
                RequestBody::Json(&body),
                WAIT,
            )
            .await
            .unwrap();
        assert_eq!(status, StatusCode::OK, "{path}：{value}\n{}", self.logs());
        value
    }

    pub fn logs(&self) -> String {
        [&self.core, &self.matrix]
            .into_iter()
            .map(|name| {
                let output = Command::new("docker")
                    .args(["logs", "--tail", "40", name])
                    .output()
                    .unwrap();
                format!(
                    "{name}:\n{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn append_aliases(args: &mut Vec<String>, service: &Value) {
    if let Some(aliases) = service
        .pointer("/networks/core/aliases")
        .and_then(Value::as_array)
    {
        for alias in aliases {
            args.extend(["--network-alias".into(), alias.as_str().unwrap().into()]);
        }
    }
}

fn append_environment(args: &mut Vec<String>, service: &Value, exclude: &[&str]) {
    for (key, value) in service["environment"].as_object().unwrap() {
        if !exclude.contains(&key.as_str()) {
            args.extend(["-e".into(), format!("{key}={}", value.as_str().unwrap())]);
        }
    }
}

fn append_mounts(args: &mut Vec<String>, service: &Value) {
    for volume in service["volumes"].as_array().unwrap() {
        if volume["type"] == "bind" {
            args.extend([
                "-v".into(),
                format!(
                    "{}:{}{}",
                    volume["source"].as_str().unwrap(),
                    volume["target"].as_str().unwrap(),
                    if volume["read_only"] == true {
                        ":ro"
                    } else {
                        ""
                    }
                ),
            ]);
        }
    }
}

/// 测试进程访问这台 Tuwunel 的地址。
///
/// 测试进程在 Docker 的宿主上，直接连容器在测试网络里的地址：宿主上不发布端口，Docker 就不会挑到一个正被
/// 别的套接字占着的号（配上就是 `address already in use`，和并行测试抢临时端口段，2026-11-10 的 CI 红在
/// 这里）。Docker Desktop、OrbStack 上宿主到不了容器地址，仍发布一个宿主端口再用它。
fn matrix_url(container: &str, network: &str) -> String {
    if cfg!(target_os = "linux") {
        let ip = docker(&[
            "inspect",
            "--format",
            &format!("{{{{(index .NetworkSettings.Networks \"{network}\").IPAddress}}}}"),
            container,
        ]);
        return format!("http://{ip}:8008");
    }
    let port = docker(&["port", container, "8008/tcp"]);
    format!("http://{}", port.lines().next().unwrap())
}

fn docker<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> String {
    let output = Command::new("docker").args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{:?}: {}{}",
        args.iter().map(AsRef::as_ref).collect::<Vec<_>>(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
