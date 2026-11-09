//! 出口容器（ADR-0023 的“出口容器”）：gost 的配置和容器本身。核心经 Docker API 管理它，因为它要随 App 里的设置出现和消失。

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, chown};
use std::path::{Path, PathBuf};

use bollard::models::{
    ContainerCreateBody, HostConfig, NetworkCreateRequest, RestartPolicy, RestartPolicyNameEnum,
};
use ipnet::Ipv4Net;
use rustix::process::{getegid, geteuid};
use serde::Serialize;

use super::address::{ProxyAddress, Scheme};
use super::lists::Lists;
use crate::plugin::docker::{Engine, EngineError};
use crate::plugin::hostdir::Owner;

/// gost 3.3.3，MIT，独立容器。
pub const IMAGE: &str =
    "gogost/gost:3@sha256:f266f957c6e4f8d433ed23202a5745fa4969d08f3f0ef034fb9357345e16ad67";

/// CI 把测试镜像复制到自己的仓库后经 `WONTON_TEST_IMAGES` 指过去（wonton-test-images）；
/// 没设时就是 [`IMAGE`]，本地行为不变。
pub fn image() -> String {
    wonton_test_images::image(IMAGE)
}

/// 插件网络里的别名。没声明出网的插件网络里没有它，也就连不到出口容器。
pub const ALIAS: &str = "egress.wonton.internal";

/// 容器里挂配置的地方。
const MOUNT: &str = "/etc/gost";
const CONFIG_FILE: &str = "gost.yaml";
const DENY_FILE: &str = "deny.txt";
const DIRECT_FILE: &str = "direct.txt";
const LISTEN: &str = ":3128";
/// 名单每隔这么久重新读一次，不用重建容器，正在进行的连接不断（实测）。
const RELOAD: &str = "3s";

const NOBODY: Owner = Owner {
    uid: 65534,
    gid: 65534,
};

#[derive(Serialize)]
struct Config {
    services: [Service; 1],
    chains: [Chain; 1],
    bypasses: [Bypass; 2],
}

#[derive(Serialize)]
struct Service {
    name: &'static str,
    addr: &'static str,
    bypass: &'static str,
    handler: Handler,
    listener: Plain,
}

#[derive(Serialize)]
struct Handler {
    r#type: &'static str,
    chain: &'static str,
}

#[derive(Serialize)]
struct Plain {
    r#type: &'static str,
}

#[derive(Serialize)]
struct Chain {
    name: &'static str,
    hops: [Hop; 1],
}

#[derive(Serialize)]
struct Hop {
    name: &'static str,
    nodes: [Node; 1],
}

#[derive(Serialize)]
struct Node {
    name: &'static str,
    addr: String,
    bypass: &'static str,
    connector: Connector,
    dialer: Dialer,
}

#[derive(Serialize)]
struct Connector {
    r#type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<Auth>,
}

#[derive(Serialize)]
struct Auth {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct Dialer {
    r#type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tls: Option<Tls>,
}

#[derive(Serialize)]
struct Tls {
    secure: bool,
    #[serde(rename = "serverName")]
    server_name: String,
}

#[derive(Serialize)]
struct Bypass {
    name: &'static str,
    reload: &'static str,
    file: BypassFile,
}

#[derive(Serialize)]
struct BypassFile {
    path: String,
}

fn bypass(name: &'static str, file: &str) -> Bypass {
    Bypass {
        name,
        reload: RELOAD,
        file: BypassFile {
            path: format!("{MOUNT}/{file}"),
        },
    }
}

/// gost 的配置。用 YAML 库生成，不拼字符串：密码可以是任何字符。
use super::super::languages::Languages;

pub fn config_yaml(proxy: &ProxyAddress, langs: &Languages) -> Result<String, String> {
    let auth = proxy.username.is_some() || proxy.password.is_some();
    let auth = auth.then(|| Auth {
        username: proxy.username.clone().unwrap_or_default(),
        password: proxy.password.clone().unwrap_or_default(),
    });
    let connector = match proxy.scheme {
        Scheme::Http | Scheme::Https => "http",
        Scheme::Socks5 => "socks5",
    };
    let dialer = match proxy.scheme {
        Scheme::Http | Scheme::Socks5 => Dialer {
            r#type: "tcp",
            tls: None,
        },
        // gost 默认不校验证书；代理的密码要经这条连接。
        Scheme::Https => Dialer {
            r#type: "tls",
            tls: Some(Tls {
                secure: true,
                server_name: proxy.host.clone(),
            }),
        },
    };
    let config = Config {
        services: [Service {
            name: "egress",
            addr: LISTEN,
            bypass: "deny",
            handler: Handler {
                r#type: "http",
                chain: "upstream",
            },
            listener: Plain { r#type: "tcp" },
        }],
        chains: [Chain {
            name: "upstream",
            hops: [Hop {
                name: "owner",
                nodes: [Node {
                    name: "proxy",
                    addr: proxy.authority(),
                    bypass: "direct",
                    connector: Connector {
                        r#type: connector,
                        auth,
                    },
                    dialer,
                }],
            }],
        }],
        bypasses: [bypass("deny", DENY_FILE), bypass("direct", DIRECT_FILE)],
    };
    serde_saphyr::to_string(&config).map_err(|e| {
        crate::l10n::text(
            langs,
            "plugin-settings-egress-gost-config",
            &[("error", e.to_string().into())],
        )
    })
}

/// 运行 gost 的用户：核心是 root 时是 nobody；不是 root（开发时）时只能是核心自己，文件才是它的。
pub fn runner() -> Owner {
    let uid = geteuid();
    if uid.is_root() {
        NOBODY
    } else {
        Owner {
            uid: uid.as_raw(),
            gid: getegid().as_raw(),
        }
    }
}

/// 出口容器只读挂载的目录。配置里有代理的密码：属主是运行 gost 的用户，权限 0400。容器进程的命令行宿主上的普通用户也看得到，
/// 所以不放在那里。
pub struct RelayFiles {
    dir: PathBuf,
    owner: Owner,
}

impl RelayFiles {
    pub fn new(dir: PathBuf, owner: Owner) -> Self {
        Self { dir, owner }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn prepare(&self, langs: &Languages) -> Result<(), String> {
        let context = |e: std::io::Error| {
            crate::l10n::text(
                langs,
                "plugin-settings-egress-prepare-dir",
                &[
                    ("dir", self.dir.display().to_string().into()),
                    ("error", e.to_string().into()),
                ],
            )
        };
        fs::create_dir_all(&self.dir).map_err(context)?;
        fs::set_permissions(
            &self.dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .map_err(context)?;
        self.give(&self.dir).map_err(context)
    }

    fn give(&self, path: &Path) -> std::io::Result<()> {
        if geteuid().is_root() {
            chown(path, Some(self.owner.uid), Some(self.owner.gid))?;
        }
        Ok(())
    }

    /// 内容变了才写：写临时文件再改名，gost 读到的总是完整的。
    fn write(&self, name: &str, data: &str, langs: &Languages) -> Result<(), String> {
        let path = self.dir.join(name);
        if fs::read(&path).is_ok_and(|current| current == data.as_bytes()) {
            return Ok(());
        }
        let context = |e: std::io::Error| {
            crate::l10n::text(
                langs,
                "plugin-settings-egress-write-file",
                &[
                    ("path", path.display().to_string().into()),
                    ("error", e.to_string().into()),
                ],
            )
        };
        let temp = self.dir.join(format!(".{name}.tmp"));
        match fs::remove_file(&temp) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(context(e)),
            _ => {}
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .open(&temp)
            .map_err(context)?;
        file.write_all(data.as_bytes()).map_err(context)?;
        file.sync_all().map_err(context)?;
        self.give(&temp).map_err(context)?;
        fs::rename(&temp, &path).map_err(context)
    }

    /// 配置和这份一样：不用重建容器。
    pub fn has_config(&self, yaml: &str) -> bool {
        fs::read(self.dir.join(CONFIG_FILE)).is_ok_and(|current| current == yaml.as_bytes())
    }

    pub fn write_config(&self, yaml: &str, langs: &Languages) -> Result<(), String> {
        self.prepare(langs)?;
        self.write(CONFIG_FILE, yaml, langs)
    }

    /// 写名单。容器不用重建。
    pub fn write_lists(&self, lists: &Lists, langs: &Languages) -> Result<(), String> {
        self.prepare(langs)?;
        self.write(DENY_FILE, &lists.deny_file(), langs)?;
        self.write(DIRECT_FILE, &lists.direct_file(), langs)
    }

    /// 取消代理时删掉配置：里面有密码。
    pub fn remove(&self, langs: &Languages) -> Result<(), String> {
        match fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(crate::l10n::text(
                langs,
                "plugin-settings-egress-remove-dir",
                &[
                    ("dir", self.dir.display().to_string().into()),
                    ("error", e.to_string().into()),
                ],
            )),
            _ => Ok(()),
        }
    }
}

/// 出口容器。
pub struct Relay<'a> {
    /// 容器名和它自己的网络名。
    pub name: &'a str,
    pub labels: &'a HashMap<String, String>,
    pub files: &'a RelayFiles,
    pub image: &'a str,
}

impl Relay<'_> {
    /// 能力全去、根文件系统只读、nobody、不转发：实测这样能工作。日志级别 `warn`，不记录插件访问了哪些网站。
    fn body(&self) -> ContainerCreateBody {
        let owner = self.files.owner;
        ContainerCreateBody {
            image: Some(self.image.to_owned()),
            cmd: Some(vec!["-C".to_owned(), format!("{MOUNT}/{CONFIG_FILE}")]),
            user: Some(format!("{}:{}", owner.uid, owner.gid)),
            env: Some(vec!["GOST_LOGGER_LEVEL=warn".to_owned()]),
            labels: Some(self.labels.clone()),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{}:{MOUNT}:ro", self.files.dir().display())]),
                network_mode: Some(self.name.to_owned()),
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    ..Default::default()
                }),
                cap_drop: Some(vec!["ALL".to_owned()]),
                security_opt: Some(vec!["no-new-privileges".to_owned()]),
                readonly_rootfs: Some(true),
                sysctls: Some(
                    [("net.ipv4.ip_forward".to_owned(), "0".to_owned())]
                        .into_iter()
                        .collect(),
                ),
                // 安装脚本把回环上的代理写成 `host.docker.internal`。
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_owned()]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

impl Engine {
    /// 出口容器自己的网络：普通桥接，默认路由在这里。
    pub async fn create_relay_network(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
    ) -> Result<(), EngineError> {
        self.create_reserved(Self::relay_request(name, labels))
            .await
    }

    /// 同 [`Self::create_relay_network`]，但在迁移预留的网段上建。
    pub async fn create_relay_network_at(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
        subnet: Ipv4Net,
    ) -> Result<(), EngineError> {
        self.create_network_at_reserved(Self::relay_request(name, labels), subnet)
            .await
    }

    fn relay_request(name: &str, labels: &HashMap<String, String>) -> NetworkCreateRequest {
        NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels.clone()),
            ..Default::default()
        }
    }

    /// 重建出口容器，约 0.6 秒（实测）。
    pub async fn start_relay(&self, relay: &Relay<'_>) -> Result<(), EngineError> {
        self.remove_container(relay.name).await?;
        self.create_container(relay.name, relay.body()).await?;
        self.start_container(relay.name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parsed(text: &str) -> Value {
        let yaml = config_yaml(
            &ProxyAddress::parse(text, &Default::default()).unwrap(),
            &Languages::default(),
        )
        .unwrap();
        serde_saphyr::from_str(&yaml).unwrap()
    }

    #[test]
    fn the_proxy_address_ends_up_in_the_node_and_nowhere_else() {
        let config = parsed("http://proxy:7899");
        let node = &config["chains"][0]["hops"][0]["nodes"][0];
        assert_eq!(node["addr"], "proxy:7899");
        assert_eq!(node["bypass"], "direct");
        assert_eq!(node["connector"]["type"], "http");
        assert_eq!(node["connector"].get("auth"), None);
        assert_eq!(node["dialer"]["type"], "tcp");
        assert_eq!(config["services"][0]["bypass"], "deny");
        assert_eq!(config["services"][0]["handler"]["chain"], "upstream");
        assert_eq!(config["bypasses"][0]["file"]["path"], "/etc/gost/deny.txt");
        assert_eq!(config["bypasses"][1]["reload"], "3s");
    }

    #[test]
    fn socks_and_https_proxies_connect_differently() {
        let node = |text| parsed(text)["chains"][0]["hops"][0]["nodes"][0].clone();
        assert_eq!(node("socks5://proxy:1080")["connector"]["type"], "socks5");
        let https = node("https://proxy.example.com");
        assert_eq!(https["connector"]["type"], "http");
        assert_eq!(https["dialer"]["type"], "tls");
        assert_eq!(https["dialer"]["tls"]["secure"], true);
        assert_eq!(https["dialer"]["tls"]["serverName"], "proxy.example.com");
    }

    #[test]
    fn a_password_with_yaml_syntax_in_it_survives() {
        let config = parsed("http://u%40x:*p%3A%20%23%26%22@proxy:7899");
        let auth = &config["chains"][0]["hops"][0]["nodes"][0]["connector"]["auth"];
        assert_eq!(auth["username"], "u@x");
        assert_eq!(auth["password"], "*p: #&\"");
    }

    #[test]
    fn files_are_private_and_replaced_whole() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let files = RelayFiles::new(dir.path().join("relay"), runner());
        assert!(!files.has_config("a: 1\n"));
        files.write_config("a: 1\n", &Languages::default()).unwrap();
        assert!(files.has_config("a: 1\n"));
        files.write_config("a: 2\n", &Languages::default()).unwrap();
        let path = files.dir().join(CONFIG_FILE);
        assert_eq!(fs::read_to_string(&path).unwrap(), "a: 2\n");
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o400);
        assert_eq!(mode(files.dir()), 0o700);
        let lists = Lists {
            deny: vec!["10.0.0.0/8".to_owned(), "*.x".to_owned()],
            ..Lists::default()
        };
        files.write_lists(&lists, &Languages::default()).unwrap();
        let deny = fs::read_to_string(files.dir().join(DENY_FILE)).unwrap();
        assert_eq!(deny, "10.0.0.0/8\n*.x\n");
        assert_eq!(
            fs::read_to_string(files.dir().join(DIRECT_FILE)).unwrap(),
            ""
        );
        files.remove(&Languages::default()).unwrap();
        assert!(!files.dir().exists());
        files.remove(&Languages::default()).unwrap();
    }
}
