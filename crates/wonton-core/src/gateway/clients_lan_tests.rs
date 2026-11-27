//! 局域网入口（ADR-0021“局域网直连”、ADR-0089 第 4 节）：局域网转发发布在哪、放行谁、地址变了重建谁，
//! `*.lan` 记录不对时怎么报、改好后怎么恢复。

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use hyper::{Method, StatusCode};
use serde_json::Value;

use super::Rig;
use crate::gateway::family_tests::send_as;
use crate::gateway::tests::{CORE, TOKEN};
use crate::plugin::lan::probe::{DefaultRoute, HostAddress, HostInterface, HostNetwork};

const FORWARD: &str = "wonton-lan-forward";

/// 假解析器：A 查询按 `answers` 回，空的回 NXDOMAIN，别的类型也回 NXDOMAIN。
pub(super) struct FakeDns {
    pub(super) server: String,
    answers: Arc<Mutex<Vec<Ipv4Addr>>>,
}

impl FakeDns {
    pub(super) async fn start(answers: &[&str]) -> Self {
        use hickory_resolver::proto::op::{Message, OpCode, ResponseCode};
        use hickory_resolver::proto::rr::{RData, Record, RecordType, rdata::A};
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server = socket.local_addr().unwrap().to_string();
        let dns = Self {
            server,
            answers: Arc::new(Mutex::new(Vec::new())),
        };
        dns.answer(answers);
        let current = dns.answers.clone();
        tokio::spawn(async move {
            let mut bytes = [0; 2048];
            while let Ok((length, peer)) = socket.recv_from(&mut bytes).await {
                let Ok(query) = Message::from_vec(&bytes[..length]) else {
                    continue;
                };
                let question = query.queries[0].clone();
                let mut reply = Message::response(query.metadata.id, OpCode::Query);
                reply.queries.push(question.clone());
                let answers = current.lock().unwrap().clone();
                if question.query_type == RecordType::A && !answers.is_empty() {
                    for address in answers {
                        reply.answers.push(Record::from_rdata(
                            question.name.clone(),
                            60,
                            RData::A(A(address)),
                        ));
                    }
                } else {
                    reply.metadata.response_code = ResponseCode::NXDomain;
                }
                let _ = socket.send_to(&reply.to_vec().unwrap(), peer).await;
            }
        });
        dns
    }

    /// 换掉之后的回答，空的就是没有记录。
    pub(super) fn answer(&self, answers: &[&str]) {
        *self.answers.lock().unwrap() = answers.iter().map(|a| a.parse().unwrap()).collect();
    }
}

/// 宿主只有一张走默认路由的有线网卡 eth0，地址 `address/24`。
fn host(address: &str) -> HostNetwork {
    HostNetwork {
        routes: vec![DefaultRoute {
            interface: "eth0".into(),
            metric: 100,
        }],
        interfaces: vec![HostInterface {
            name: "eth0".into(),
            physical: true,
            addresses: vec![HostAddress {
                ip: address.parse().unwrap(),
                secondary: false,
                prefix: 24,
            }],
            ..Default::default()
        }],
        subnets: vec![],
        kernel: None,
        dmi: None,
    }
}

impl Rig {
    fn dns(&self) -> &FakeDns {
        self.dns.as_ref().expect("lan 为真的 Rig 才有假解析器")
    }

    /// 宿主换成 `address`，`*.lan` 也解析到它，再检测一次。
    async fn move_lan_to(&self, address: &str) {
        self.h.manager.set_lan_host_for_test(host(address));
        self.dns().answer(&[address]);
        self.h.manager.lan_pass_for_test().await;
    }

    fn forward_created(&self) -> Option<Value> {
        self.h.docker.created(FORWARD)
    }

    fn forward_config(&self) -> String {
        std::fs::read_to_string(self.install.join("entry/lan-forward.cfg")).unwrap()
    }

    /// 局域网转发在宿主上发布的地址和端口，`(HostIp, HostPort)`，按端口分组。
    fn forward_ports(&self) -> Vec<(String, String, String)> {
        let created = self.forward_created().expect("局域网转发建了");
        let bindings = created["HostConfig"]["PortBindings"].as_object().unwrap();
        bindings
            .iter()
            .flat_map(|(port, hosts)| {
                hosts.as_array().unwrap().iter().map(move |host| {
                    (
                        port.clone(),
                        host["HostIp"].as_str().unwrap_or_default().to_owned(),
                        host["HostPort"].as_str().unwrap_or_default().to_owned(),
                    )
                })
            })
            .collect()
    }

    async fn lan_record(&self) -> Value {
        let bearer = format!("Bearer {TOKEN}");
        let reply = send_as(
            &self.app,
            None,
            Method::GET,
            CORE,
            &wonton_protocol::clients_domain_path(),
            &[("authorization", &bearer), ("x-wonton-clients", "1")],
            "",
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        reply.json()["lan_record"].clone()
    }
}

#[tokio::test]
async fn lan_forward_published_on_selected_ipv4_only() {
    let rig = Rig::start(true, true, true).await;
    assert!(rig.forward_created().is_none(), "没人开局域网入口时不建");
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(
        rig.forward_ports(),
        vec![("8483/tcp".into(), "192.168.1.10".into(), "8483".into())],
        "只发布在选中的 IPv4 上，eth1 的 IPv6 和通配地址都不发布"
    );
    let created = rig.forward_created().unwrap();
    let host = &created["HostConfig"];
    assert_ne!(created["User"].as_str().unwrap_or("root"), "root");
    assert_eq!(host["CapDrop"], serde_json::json!(["ALL"]));
    assert_eq!(host["ReadonlyRootfs"], true);
    assert!(
        !created.to_string().contains("docker.sock"),
        "转发不碰 Docker：{created}"
    );
    assert!(
        rig.forward_config()
            .contains("reject unless { src 192.168.1.0/24 }"),
        "{}",
        rig.forward_config()
    );
    let lanpub = rig.h.docker.network("wonton_lanpub").expect("lanpub 建了");
    assert_eq!(
        lanpub["Options"]["com.docker.network.bridge.enable_ip_masquerade"],
        "false"
    );
    let lanfwd = rig.h.docker.network("wonton_lanfwd").expect("lanfwd 建了");
    assert_eq!(lanfwd["Internal"], true);
    for network in [&lanpub, &lanfwd] {
        assert_eq!(network["Labels"]["wonton.managed-by"], "core", "{network}");
    }
    assert!(
        rig.h
            .docker
            .endpoint("wonton_lanfwd", "wonton-entry")
            .is_some(),
        "入口接在 lanfwd 上"
    );
    assert!(
        rig.h
            .docker
            .endpoint("wonton_lanfwd", "wonton-connector")
            .is_none(),
        "连接器不在 lanfwd 上"
    );
}

#[tokio::test]
async fn announce_binds_any_in_vm() {
    let rig = Rig::start(true, true, true).await;
    rig.dns().answer(&["192.168.64.5"]);
    rig.h
        .manager
        .lan_pass_announced_for_test("192.168.64.5/24")
        .await;
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(
        rig.forward_ports(),
        vec![("8483/tcp".into(), "0.0.0.0".into(), "8483".into())]
    );
    assert!(
        rig.forward_config()
            .contains("reject unless { src 192.168.64.0/24 }"),
        "{}",
        rig.forward_config()
    );
}

#[tokio::test]
async fn non_rfc1918_selected_refuses() {
    let rig = Rig::start(true, true, true).await;
    rig.move_lan_to("100.64.0.10").await;
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error_kind(), Some("lan_unavailable"));
    assert!(rig.forward_created().is_none());
    assert!(
        !rig.h
            .manager
            .record_for_test(&rig.id)
            .unwrap()
            .clients_entries
            .lan
    );
}

#[tokio::test]
async fn gateway_address_not_in_allowed_source() {
    let rig = Rig::start(true, true, true).await;
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let lanpub = rig.h.docker.network("wonton_lanpub").unwrap();
    let gateway = lanpub["IPAM"]["Config"][0]["Gateway"]
        .as_str()
        .expect("lanpub 有网关")
        .to_owned();
    let config = rig.forward_config();
    let deny = config
        .find(&format!("reject if {{ src {gateway} }}"))
        .unwrap_or_else(|| panic!("网关 {gateway} 没被单独拒掉：{config}"));
    let allow = config.find("reject unless").unwrap();
    assert!(deny < allow, "先拒网关再按网段放行：{config}");
}

#[tokio::test]
async fn address_change_recreates_only_lan_forward() {
    let rig = Rig::start(true, true, true).await;
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let docker = &rig.h.docker;
    let before = (
        docker.creations("wonton-entry"),
        docker.creations("wonton-connector"),
        docker.creations(FORWARD),
    );
    let reloads = rig.reloads();
    rig.move_lan_to("192.168.2.10").await;
    assert_eq!(
        rig.forward_ports(),
        vec![("8483/tcp".into(), "192.168.2.10".into(), "8483".into())]
    );
    assert!(rig.forward_config().contains("src 192.168.2.0/24"));
    assert_eq!(docker.creations(FORWARD), before.2 + 1);
    assert_eq!(docker.creations("wonton-entry"), before.0, "入口不重建");
    assert_eq!(
        docker.creations("wonton-connector"),
        before.1,
        "连接器不重建"
    );
    assert_eq!(rig.reloads(), reloads, "入口不重载");
    // 地址没变再检测一次：什么都不动。
    rig.h.manager.lan_pass_for_test().await;
    assert_eq!(docker.creations(FORWARD), before.2 + 1);
}

#[tokio::test]
async fn lan_record_mismatch_reports_lan_unavailable_with_record() {
    let rig = Rig::start(true, true, true).await;
    rig.dns().answer(&["192.168.1.99"]);
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error_kind(), Some("lan_unavailable"));
    assert!(
        reply
            .text()
            .contains("*.lan.home.example.test A 192.168.1.10"),
        "要给出该加的记录：{}",
        reply.text()
    );
    assert_eq!(
        rig.lan_record().await,
        serde_json::json!({"state": "wrong", "found": "192.168.1.99"})
    );
    assert!(rig.forward_created().is_none());

    rig.dns().answer(&[]);
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.error_kind(), Some("lan_unavailable"));
    assert!(
        reply
            .text()
            .contains("*.lan.home.example.test A 192.168.1.10")
    );
    assert_eq!(
        rig.lan_record().await,
        serde_json::json!({"state": "missing"})
    );
}

#[tokio::test]
async fn lan_record_fixed_recovers() {
    let rig = Rig::start(true, true, true).await;
    rig.dns().answer(&["192.168.1.99"]);
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.error_kind(), Some("lan_unavailable"));
    assert_eq!(rig.lan_record().await["state"], "wrong");
    // 主人改好记录：下一次定期检测重新查，不用等地址变。
    rig.dns().answer(&["192.168.1.10"]);
    rig.h.manager.lan_pass_for_test().await;
    assert_eq!(rig.lan_record().await, serde_json::json!({"state": "ok"}));
    let reply = rig.switch("lan", true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(
        rig.forward_ports(),
        vec![("8483/tcp".into(), "192.168.1.10".into(), "8483".into())]
    );
}
