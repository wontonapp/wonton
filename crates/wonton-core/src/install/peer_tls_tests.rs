//! 配对联邦的证书材料（ADR-0112）：三种安装形态都会落盘，私钥 0600，信任包是系统根加本机 CA，
//! Tuwunel 挂信任包、按它验证核心，`allow_invalid` 只在有老配对时留窗口。

use std::fs;
use std::path::{Path, PathBuf};

use rcgen::KeyPair;
use serde_json::Value;
use x509_parser::pem::parse_x509_pem;

use crate::peer_tls::{ca_issuer, leaf_params};

use super::entry_mode_tests::named_settings;
use super::*;

const IMAGE: &str = "ghcr.io/wontonapp/wonton-core@sha256:0123456789abcdef";

fn peer_dir(dir: &Path) -> PathBuf {
    dir.join("secrets/peer-tls")
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// 证书解析结果借不来整份文件，取测试要看的字段存下来。
struct Cert {
    der: Vec<u8>,
    subject: Vec<u8>,
    issuer: Vec<u8>,
    san: Vec<String>,
    is_ca: bool,
    days: i64,
}

impl Cert {
    fn read(dir: &Path, name: &str) -> Self {
        let bytes = fs::read(peer_dir(dir).join(name)).unwrap();
        let pem = parse_x509_pem(&bytes).unwrap().1;
        let der = pem.contents;
        let (_, cert) = x509_parser::parse_x509_certificate(&der).unwrap();
        let validity = cert.validity();
        Self {
            subject: cert.subject().as_raw().to_vec(),
            issuer: cert.issuer().as_raw().to_vec(),
            san: cert
                .subject_alternative_name()
                .ok()
                .flatten()
                .map(|san| {
                    san.value
                        .general_names
                        .iter()
                        .filter_map(|name| match name {
                            x509_parser::extensions::GeneralName::DNSName(name) => {
                                Some((*name).to_owned())
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            is_ca: cert.is_ca(),
            days: (validity.not_after.timestamp() - validity.not_before.timestamp()) / 86_400,
            der,
        }
    }

    /// 用签发者的公钥验这张证书，没有签发者时按自签验。
    fn verify(&self, issuer: Option<&Self>) {
        let (_, cert) = x509_parser::parse_x509_certificate(&self.der).unwrap();
        match issuer {
            Some(issuer) => {
                let (_, issuer) = x509_parser::parse_x509_certificate(&issuer.der).unwrap();
                cert.verify_signature(Some(issuer.public_key())).unwrap();
            }
            None => cert.verify_signature(None).unwrap(),
        }
    }
}

fn compose(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn install(dir: &Path, name: Option<&str>, data: &Path) -> Report {
    init_with(dir, name, Some(data), Some(IMAGE)).unwrap()
}

/// 三种安装形态（有入口、有名字没入口、私用名）都落同样的材料；Tuwunel 不再自己挂证书。
#[test]
fn every_install_form_writes_the_same_peer_tls_material() {
    let cases: [(&str, Option<&str>, bool); 3] = [
        ("entry", Some("home.example"), true),
        ("no_entry", Some("home.example"), false),
        ("private", Some("home.wonton.internal"), false),
    ];
    for (label, name, entry) in cases {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        if entry {
            named_settings(dir.path());
        }
        let report = install(dir.path(), name, data.path());
        let material = peer_dir(dir.path());
        let server = name.unwrap();

        for file in [
            "ca.pem",
            "ca.key",
            "peer.pem",
            "peer.key",
            "trust.pem",
            "system-roots.pem",
        ] {
            assert!(material.join(file).exists(), "{label}: {file} 没生成");
        }
        for key in ["ca.key", "peer.key"] {
            assert_eq!(mode(&material.join(key)), 0o600, "{label}: {key} 的权限");
        }
        assert_eq!(mode(&material.join("trust.pem")), 0o644, "{label}: 信任包");
        assert_eq!(mode(&material), 0o700, "{label}: 目录");
        assert_eq!(
            fs::read_to_string(material.join("system-roots.source")).unwrap(),
            format!("{IMAGE}\n")
        );

        // 信任包整份等于系统根接上本机 CA，没有别的东西。
        let roots = fs::read_to_string(material.join("system-roots.pem")).unwrap();
        let ca = fs::read_to_string(material.join("ca.pem")).unwrap();
        assert!(
            roots.matches("BEGIN CERTIFICATE").count() > 1,
            "{label}: 系统根是空的"
        );
        assert_eq!(
            fs::read_to_string(material.join("trust.pem")).unwrap(),
            format!("{roots}{ca}")
        );

        let config = fs::read_to_string(dir.path().join("secrets/tuwunel/tuwunel.toml")).unwrap();
        assert!(
            !config.contains("[global.tls]"),
            "{label}: Tuwunel 不再自己听 TLS"
        );
        assert!(
            config.contains("allow_invalid_tls_certificates = false"),
            "{label}: 全新安装不放宽校验\n{config}"
        );
        for file in ["tuwunel-tls.crt", "tuwunel-tls.key"] {
            assert!(
                !dir.path().join("secrets/tuwunel").join(file).exists(),
                "{label}: 旧的自签 {file} 还留着"
            );
        }

        let compose = compose(&dir.path().join("compose.yaml"));
        assert_eq!(
            compose["services"]["tuwunel"]["environment"]["SSL_CERT_FILE"],
            "/etc/wonton-peer-tls/trust.pem"
        );
        let mounts = compose["services"]["tuwunel"]["volumes"]
            .as_array()
            .unwrap();
        assert!(
            mounts
                .iter()
                .any(|volume| volume
                    == "${WONTON_INSTALL_DIR}/secrets/peer-tls:/etc/wonton-peer-tls:ro"),
            "{label}: 信任包没挂进 Tuwunel：{mounts:?}"
        );
        assert_eq!(
            compose["services"]["core"]["environment"]["WONTON_PEER_TLS_DIR"],
            "${WONTON_INSTALL_DIR}/secrets/peer-tls"
        );
        assert!(
            report.created.contains(&material.join("trust.pem")),
            "{label}: 信任包不在生成列表里"
        );

        // 证书名对得上服务器名。
        let peer = Cert::read(dir.path(), "peer.pem");
        assert_eq!(peer.san, [server]);
    }
}

/// 有效期 5 年；对外那张是自签叶，给本机 Tuwunel 的那张由本机 CA 签、验得过。
#[test]
fn the_material_has_five_year_leaves_and_a_working_local_chain() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    install(dir.path(), Some("home.example"), data.path());

    let ca = Cert::read(dir.path(), "ca.pem");
    assert!(ca.is_ca, "CA 要带 CA 标记");
    let peer = Cert::read(dir.path(), "peer.pem");
    assert!(!peer.is_ca, "对外的是叶证书");
    assert_eq!(peer.issuer, peer.subject, "对外的是自签叶");
    assert_eq!(peer.san, ["home.example"]);
    peer.verify(None);

    // 核心按 CONNECT 目标名现签给本机 Tuwunel 的证书：链到本机 CA，名字是目标名。
    let issuer = ca_issuer(&peer_dir(dir.path())).unwrap();
    let pair = KeyPair::generate().unwrap();
    let probe_pem = leaf_params("b.example.internal")
        .unwrap()
        .signed_by(&pair, &issuer)
        .unwrap()
        .pem();
    fs::write(peer_dir(dir.path()).join("probe.pem"), probe_pem).unwrap();
    let probe = Cert::read(dir.path(), "probe.pem");
    assert_eq!(probe.issuer, ca.subject, "现签的证书由本机 CA 签");
    assert_eq!(probe.san, ["b.example.internal"]);
    probe.verify(Some(&ca));

    for cert in [&ca, &peer, &probe] {
        assert!(
            (1824..=1830).contains(&cert.days),
            "有效期 {} 天，不是五年",
            cert.days
        );
    }
}

/// 重跑不换密钥；换了镜像重抄系统根并更新来源，信任包跟着重写。
#[test]
fn a_rerun_keeps_the_keys_and_refreshes_the_recorded_source() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    install(dir.path(), Some("home.example"), data.path());
    let before =
        ["ca.key", "peer.pem"].map(|file| fs::read(peer_dir(dir.path()).join(file)).unwrap());

    let report = init_with(
        dir.path(),
        Some("home.example"),
        Some(data.path()),
        Some("ghcr.io/wontonapp/wonton-core@sha256:feedface"),
    )
    .unwrap();

    for (file, content) in ["ca.key", "peer.pem"].iter().zip(before) {
        assert_eq!(fs::read(peer_dir(dir.path()).join(file)).unwrap(), content);
        assert!(!report.created.contains(&peer_dir(dir.path()).join(file)));
    }
    assert_eq!(
        fs::read_to_string(peer_dir(dir.path()).join("system-roots.source")).unwrap(),
        "ghcr.io/wontonapp/wonton-core@sha256:feedface\n"
    );
}

/// 有配对了还没钉住的记录时留着放宽的窗口；都钉住了（或还没有配对）就不放宽。
#[test]
fn only_unpinned_paired_records_keep_the_legacy_window_open() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = data.path().join("core/peers.json");
    fs::create_dir_all(store.parent().unwrap()).unwrap();
    let config = || fs::read_to_string(dir.path().join("secrets/tuwunel/tuwunel.toml")).unwrap();
    let paired = |pin_state: &str, pin: &str| {
        format!(
            r#"{{"peers":{{"old.example":{{"server_name":"old.example","endpoint_id":"{}","owner_user_id":"@o:old.example","owner_display_name":"Old","paired_at_ms":1,"phase":"paired"{pin_state}{pin}}}}}}}"#,
            "a".repeat(52)
        )
    };

    fs::write(
        &store,
        paired(r#","pin_state":{"pending":{"since_version":"0.8.0"}}"#, ""),
    )
    .unwrap();
    install(dir.path(), Some("home.example"), data.path());
    assert!(
        config().contains("allow_invalid_tls_certificates = true"),
        "待钉的配对要留窗口"
    );

    fs::write(
        &store,
        paired(
            r#","pin_state":"pinned""#,
            r#","peer_pin":[{"der":"MIIB","spki_sha256":"00","pinned_at_ms":1}]"#,
        ),
    )
    .unwrap();
    init_with(dir.path(), Some("home.example"), Some(data.path()), None).unwrap();
    assert!(
        config().contains("allow_invalid_tls_certificates = false"),
        "钉齐了就关窗口"
    );

    fs::write(
        &store,
        r#"{"peers":{"new.example":{"server_name":"new.example","endpoint_id":"b"#.to_owned()
            + "b".repeat(51).as_str()
            + r#"","owner_user_id":"@o:new.example","owner_display_name":"New","paired_at_ms":1,"phase":"awaiting_peer"}}}"#,
    )
    .unwrap();
    init_with(dir.path(), Some("home.example"), Some(data.path()), None).unwrap();
    assert!(
        config().contains("allow_invalid_tls_certificates = false"),
        "还没答应的请求不算"
    );
}

/// 私用名升级：旧的自签证书（新旧两个位置）挪成对外的叶，不再留给 Tuwunel 自己听。
#[test]
fn an_old_self_signed_cert_becomes_the_outward_leaf() {
    for old in ["tuwunel", ""] {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        install(dir.path(), Some("home.wonton.internal"), data.path());
        let secret = dir.path().join("secrets");
        let slot = if old.is_empty() {
            secret.clone()
        } else {
            secret.join(old)
        };
        // 模拟升级前的安装：把材料挪到旧位置，旧位置才是升级前的样子。
        for name in ["peer.pem", "peer.key"] {
            fs::remove_file(peer_dir(dir.path()).join(name)).unwrap();
        }
        fs::write(slot.join("tuwunel-tls.crt"), "OLD CERT").unwrap();
        fs::write(slot.join("tuwunel-tls.key"), "OLD KEY").unwrap();

        install(dir.path(), None, data.path());

        assert_eq!(
            fs::read_to_string(peer_dir(dir.path()).join("peer.pem")).unwrap(),
            "OLD CERT",
            "{old:?}"
        );
        assert_eq!(
            fs::read_to_string(peer_dir(dir.path()).join("peer.key")).unwrap(),
            "OLD KEY",
            "{old:?}"
        );
        assert!(!slot.join("tuwunel-tls.crt").exists());
        assert!(!slot.join("tuwunel-tls.key").exists());
    }
}

/// 一键升级在容器里跑 init，没有环境变量：数据目录和镜像从安装目录 `.env` 里读，窗口和来源照样对。
#[test]
fn an_upgrade_reads_the_data_dir_and_image_from_the_recorded_env() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = data.path().join("core/peers.json");
    fs::create_dir_all(store.parent().unwrap()).unwrap();
    fs::write(
        &store,
        format!(
            r#"{{"peers":{{"old.example":{{"endpoint_id":"{}","owner_user_id":"@o:old.example","owner_display_name":"Old","paired_at_ms":1,"phase":"paired"}}}}}}"#,
            "a".repeat(52)
        ),
    )
    .unwrap();
    fs::write(
        dir.path().join(".env"),
        format!(
            "WONTON_DATA_DIR={}\nWONTON_CORE_IMAGE=from-env\n",
            data.path().display()
        ),
    )
    .unwrap();

    init_with(dir.path(), Some("home.example"), None, None).unwrap();

    let config = fs::read_to_string(dir.path().join("secrets/tuwunel/tuwunel.toml")).unwrap();
    assert!(
        config.contains("allow_invalid_tls_certificates = true"),
        "{config}"
    );
    assert_eq!(
        fs::read_to_string(peer_dir(dir.path()).join("system-roots.source")).unwrap(),
        "from-env\n"
    );
}

/// 复审补测：`legacy_window` 读的是 1121 真实写出的记录形状（`peer_pin` 是对象数组、`pin_state` 是
/// externally tagged 的 `{"pending":…}`），不是这里测试手写的字符串占位。解析失败会让升级时 `init`
/// 直接报错。窗口该开着（有待钉记录）。
#[test]
fn review_legacy_window_reads_the_real_pin_shape() {
    let data = tempfile::tempdir().unwrap();
    let store = data.path().join("core/peers.json");
    fs::create_dir_all(store.parent().unwrap()).unwrap();
    let record = serde_json::json!({
        "peers": {
            "old.example": {
                "server_name": "old.example",
                "endpoint_id": "a".repeat(52),
                "relays": [],
                "addrs": [],
                "owner_user_id": "@o:old.example",
                "owner_display_name": "Old",
                "paired_at_ms": 1,
                "phase": "paired",
                "peer_pin": [{
                    "der": "MIIB",
                    "spki_sha256": "00".repeat(32),
                    "pinned_at_ms": 1
                }],
                "pin_state": {"pending": {"since_version": "0.8.0"}}
            }
        }
    });
    fs::write(&store, record.to_string()).unwrap();
    assert!(
        legacy_window(Some(data.path())).unwrap(),
        "有待钉记录时窗口该开着"
    );
}
