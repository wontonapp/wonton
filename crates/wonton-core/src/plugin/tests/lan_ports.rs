//! 局域网端口被宿主上已有的东西占着（#631）：预览里写明被谁占着，安装一个容器都不建就失败；自己的容器、停着的容器、
//! 别的地址、别的协议都不算冲突；Docker 在启动上因为端口被占拒绝的原话有单独的类别。

use serde_json::{Value, json};

use super::super::PluginPackage;
use super::super::docker::EngineError;
use super::super::install_error::InstallError;
use super::super::languages::Languages;
use super::lan::{granted, office, on, on_with_failing_probe, samba};
use crate::l10n::CATALOG;

/// 不是核心装的容器占着端口。
fn stranger_on(port: &str, host_ip: &str, protocol: &str) -> Value {
    json!({"HostConfig": {"PortBindings": {format!("{port}/{protocol}"): [{"HostIp": host_ip, "HostPort": port}]}}})
}

#[tokio::test]
async fn a_port_published_by_a_stranger_container_is_refused_before_anything_starts() {
    for (port, host_ip, protocol) in [
        ("445", "0.0.0.0", "tcp"),
        ("137", "::", "udp"),
        ("445", "192.168.0.10", "tcp"),
    ] {
        let (docker, m, _, _dir) = on(office()).await;
        docker.add_stranger_container("forgejo", stranger_on(port, host_ip, protocol));
        let compose = samba("");
        let package = PluginPackage::parse(&compose, &super::langs()).unwrap();
        let preview = m
            .preview_install(&package, &Languages::default())
            .await
            .unwrap();
        let blocked = preview.blocked.expect("端口被占");
        assert!(
            blocked.contains(&format!(":{}", port)) && blocked.contains("forgejo"),
            "{host_ip}:{port}/{protocol}：{blocked}"
        );
        let before = docker.writes().len();
        let err = m
            .install(&compose, &granted(&compose), super::langs())
            .await
            .unwrap_err();
        assert_eq!(
            err.kind(),
            "port_taken",
            "{host_ip}:{port}/{protocol}：{err}"
        );
        assert!(err.to_string().contains(&format!(":{}", port)), "{err}");
        assert_eq!(docker.writes().len(), before, "查出冲突时什么也不建");
    }
}

#[tokio::test]
async fn a_port_the_plugin_itself_holds_or_nothing_holds_is_no_conflict() {
    let (docker, m, _, _dir) = on(office()).await;
    // 别的地址上的同一个端口、停着的容器、别的协议：都不算。
    docker.add_stranger_container("wifi", stranger_on("445", "192.168.0.11", "tcp"));
    docker.add_stranger_container("stopped", stranger_on("445", "0.0.0.0", "tcp"));
    docker.set_state("stopped", json!({"Status": "exited", "Running": false}));
    docker.add_stranger_container("dns", stranger_on("137", "0.0.0.0", "tcp"));
    let compose = samba("");
    let package = PluginPackage::parse(&compose, &super::langs()).unwrap();
    let preview = m
        .preview_install(&package, &Languages::default())
        .await
        .unwrap();
    assert_eq!(preview.blocked, None);
    m.install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap();

    // 装着的时候自己的容器就在这个端口上：升级预览不该因为端口挡下来（还装着是别的原因）。
    let newer = compose.replace("version: 1.0.0", "version: 1.0.1");
    let preview = m
        .preview_install(
            &PluginPackage::parse(&newer, &super::langs()).unwrap(),
            &Languages::default(),
        )
        .await
        .unwrap();
    let blocked = preview.blocked.as_deref().unwrap_or_default().to_owned();
    assert!(!blocked.contains("445"), "{blocked}");
}

#[tokio::test]
async fn docker_refusing_a_start_for_a_taken_port_is_its_own_kind() {
    // 28 起多一层前缀，更早的没有；`Error starting userland proxy` 是另一种更早的写法。原话里的人话和端口要取到。
    for (message, place) in [
        (
            "failed to set up container networking: driver failed programming external connectivity on endpoint x (abc): Bind for 0.0.0.0:2222 failed: port is already allocated",
            "0.0.0.0:2222",
        ),
        (
            "failed to set up container networking: driver failed programming external connectivity on endpoint x (abc): failed to bind host port 192.168.0.69:2223/tcp: address already in use",
            "192.168.0.69:2223",
        ),
        (
            "driver failed programming external connectivity on endpoint x (abc): Bind for 0.0.0.0:2224 failed: port is already allocated",
            "0.0.0.0:2224",
        ),
        (
            "Error starting userland proxy: listen tcp4 0.0.0.0:2225: bind: address already in use",
            "0.0.0.0:2225",
        ),
    ] {
        let (docker, m, _, _dir) = on(office()).await;
        docker.add_container("x", "");
        docker.fail_starts(Some(message));
        let error = m.engine.start_container("x").await.unwrap_err();
        assert!(matches!(error, EngineError::PortTaken(_)), "{error}");
        let error = InstallError::from(error);
        assert_eq!(error.kind(), "port_taken");
        assert!(error.to_string().contains(place), "{error}");
    }
    // 只有那层前缀、没有端口的话时还是拒绝，不是端口。
    for message in [
        "driver failed programming external connectivity",
        "failed to set up container networking: driver failed programming external connectivity on endpoint x (abc)",
    ] {
        let (docker, m, _, _dir) = on(office()).await;
        docker.add_container("x", "");
        docker.fail_starts(Some(message));
        let error = m.engine.start_container("x").await.unwrap_err();
        assert_eq!(
            InstallError::from(error).kind(),
            "docker_rejected",
            "{message}"
        );
    }
}

/// 端口被占的原因按请求的语言写（ADR-0043）。
#[tokio::test]
async fn a_taken_port_follows_the_requested_language() {
    let (docker, m, _, _dir) = on(office()).await;
    docker.add_stranger_container("forgejo", stranger_on("445", "0.0.0.0", "tcp"));
    let compose = samba("");
    let mut seen = Vec::new();
    for (header, tag) in [("ja", "ja"), ("fr-FR", "en"), ("zh-TW", "zh-Hant")] {
        let error = m
            .install(
                &compose,
                &granted(&compose),
                Languages::from_header(Some(header)),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "port_taken", "{header}: {error}");
        seen.push(error.to_string());
        assert_eq!(
            error.to_string(),
            CATALOG.format(
                CATALOG.negotiate(&[tag]),
                "plugin-install-lan-port-taken",
                &[
                    ("place", "192.168.0.10:445".into()),
                    (
                        "who",
                        CATALOG
                            .format(
                                CATALOG.negotiate(&[tag]),
                                "plugin-lan-taken-by-container",
                                &[("name", "forgejo".into())]
                            )
                            .into()
                    ),
                ]
            ),
            "{header}"
        );
    }
    assert_ne!(seen[0], seen[1]);
}

/// 启动那一步被 Docker 拒绝也按主人最近一次请求的语言写：Engine 从 store 的语言文件读。
#[tokio::test]
async fn a_port_taken_at_start_follows_the_owner_language() {
    let (docker, m, _, _dir) = on(office()).await;
    docker.fail_starts(Some(
        "failed to set up container networking: driver failed programming external connectivity on endpoint x (abc):\
         Bind for 0.0.0.0:445 failed: port is already allocated",
    ));
    m.note_person_languages("owner", Some("ja"));
    let compose = samba("");
    let error = m
        .install(
            &compose,
            &granted(&compose),
            Languages::from_header(Some("ja")),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "port_taken", "{error}");
    assert!(error.to_string().contains("ポート 0.0.0.0:445"), "{error}");
}

/// 宿主端口探测失败时，预览如实写一条，别装作没人占。
#[tokio::test]
async fn a_failed_probe_says_so_in_the_preview() {
    let (_docker, m, _, _dir) = on_with_failing_probe(office(), "探测容器起不来").await;
    let compose = samba("");
    let preview = m
        .preview_install(
            &PluginPackage::parse(&compose, &super::langs()).unwrap(),
            &Languages::from_header(Some("zh-Hans")),
        )
        .await
        .unwrap();
    assert!(
        preview
            .notices
            .iter()
            .any(|n| n.text.contains("没能检查") && n.text.contains("探测容器起不来")),
        "{:?}",
        preview.notices.iter().map(|n| &n.text).collect::<Vec<_>>()
    );
}
