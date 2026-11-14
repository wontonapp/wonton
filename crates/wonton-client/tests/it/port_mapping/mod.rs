//! 插件端口在本机的同号映射（ADR-0072）：只经导出给原生壳的接口驱动，隧道对面是 `rig` 里的假服务端。

mod rig;

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use wonton_client::{HomeStatus, MappedPort, PortMappingState, PortProtocol};
use wonton_tunnel::PortResolveError;

use crate::home::support::bind_udp_until_free;
use crate::plugins::rig::{FakeHome, WAIT, running};
use rig::{App, MapHome, PLUGIN, free_port, remote};

/// 一次 UDP 往返。流向是收到第一个包才开的，回包按来源地址送回。
async fn exchange(socket: &UdpSocket, port: u16, payload: &[u8]) -> Vec<u8> {
    socket
        .send_to(payload, (Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let mut buf = vec![0u8; 65536];
    let n = tokio::time::timeout(WAIT, socket.recv(&mut buf))
        .await
        .expect("the echo did not come back")
        .unwrap();
    buf.truncate(n);
    buf
}

async fn local_udp() -> UdpSocket {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap()
}

async fn tcp_round_trip(port: u16, payload: &[u8]) -> Vec<u8> {
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    tcp.write_all(payload).await.unwrap();
    tcp.shutdown().await.unwrap();
    let mut echoed = Vec::new();
    tokio::time::timeout(WAIT, tcp.read_to_end(&mut echoed))
        .await
        .expect("the echo did not come back")
        .unwrap();
    echoed
}

async fn mapped(home: &MapHome, dir: &std::path::Path) -> App {
    let app = App::open(dir);
    app.connect(home);
    app.mapping(PortMappingState::Off).await;
    app.map(true);
    app.mapping(PortMappingState::On).await;
    app
}

#[tokio::test]
async fn a_mapped_tcp_port_reaches_the_plugin_on_the_same_number() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    let off = app.mapping(PortMappingState::Off).await;
    assert!(!off.enabled);
    assert_eq!(
        off.ports,
        [
            MappedPort {
                port: home.tcp,
                protocol: PortProtocol::Tcp
            },
            MappedPort {
                port: home.udp,
                protocol: PortProtocol::Udp
            },
        ]
    );
    app.map(true);
    assert!(app.mapping(PortMappingState::On).await.enabled);
    assert_eq!(
        tcp_round_trip(home.tcp, b"hello plugin").await,
        b"hello plugin"
    );
}

#[tokio::test]
async fn each_local_udp_source_gets_its_own_flow() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = mapped(&home, dir.path()).await;
    let (video, audio) = (local_udp().await, local_udp().await);
    assert_eq!(exchange(&video, home.udp, b"video").await, b"video");
    assert_eq!(exchange(&audio, home.udp, b"audio").await, b"audio");
    assert_eq!(exchange(&video, home.udp, b"video 2").await, b"video 2");
    let view = app.home(|view| view.details.udp_flows.len() == 2).await;
    let mut flows = view.details.udp_flows;
    flows.sort_by_key(|flow| flow.sent_packets);
    assert!(
        flows
            .iter()
            .all(|f| f.plugin_id == PLUGIN && f.port == home.udp)
    );
    assert_ne!(flows[0].flow_id, flows[1].flow_id);
    assert_eq!(
        flows
            .iter()
            .map(|f| (f.sent_packets, f.received_packets))
            .collect::<Vec<_>>(),
        [(1, 1), (2, 2)]
    );
    assert!(flows.iter().all(|f| f.buffer_evicted_drops.is_none()));
    assert_eq!(view.details.udp_buffer_evicted_datagrams, Some(0));
}

/// 2000 字节超过回环路径上的 `max_datagram_size()`，两个方向都拆两片；5000 字节放不进两片，丢掉并计数。
#[tokio::test]
async fn large_packets_are_split_and_oversized_ones_are_counted() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = mapped(&home, dir.path()).await;
    let socket = local_udp().await;
    let large: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(exchange(&socket, home.udp, &large).await, large);
    socket
        .send_to(&vec![0u8; 5000], (Ipv4Addr::LOCALHOST, home.udp))
        .await
        .unwrap();
    assert_eq!(exchange(&socket, home.udp, b"after").await, b"after");
    let view = app
        .home(|view| {
            view.details
                .udp_flows
                .iter()
                .any(|f| f.oversized_drops == 1)
        })
        .await;
    let flow = &view.details.udp_flows[0];
    assert_eq!((flow.sent_packets, flow.received_packets), (2, 2));
}

#[tokio::test]
async fn a_taken_port_is_reported_and_never_renumbered() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let taken = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, home.udp)).unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    app.mapping(PortMappingState::Off).await;
    app.map(true);
    let mapping = app.mapping(PortMappingState::PortInUse).await;
    assert!(mapping.enabled);
    assert_eq!(mapping.ports[1].port, home.udp);
    // 一个也不绑：同一插件的 TCP 端口也空着。
    drop(std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, home.tcp)).unwrap());
    drop(taken);
    app.map(false);
    app.mapping(PortMappingState::Off).await;
    app.map(true);
    app.mapping(PortMappingState::On).await;
}

#[tokio::test]
async fn an_older_server_is_reported_as_too_old() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    let device = app.core.home().device_id.parse().unwrap();
    let home = FakeHome::start(vec![device]).await;
    let (tcp, udp) = (free_port(), free_port());
    home.core.set_lists(vec![vec![remote(tcp, udp)]]);
    app.core
        .dispatch(wonton_client::Action::Home {
            action: wonton_protocol::HomeAction::ConnectHome {
                ticket: home.ticket(),
            },
        })
        .unwrap();
    app.mapping(PortMappingState::Off).await;
    app.map(true);
    app.mapping(PortMappingState::On).await;
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, tcp))
        .await
        .unwrap();
    tcp.write_all(b"x").await.unwrap();
    app.mapping(PortMappingState::ServerTooOld).await;
}

#[tokio::test]
async fn mappings_wait_out_a_tunnel_drop_and_work_again_after_it() {
    let mut home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = mapped(&home, dir.path()).await;
    let socket = local_udp().await;
    assert_eq!(exchange(&socket, home.udp, b"before").await, b"before");
    home.stop().await;
    app.mapping(PortMappingState::TunnelDisconnected).await;
    // 断开期间端口还占着，不让别的程序抢走这个号。
    assert!(std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, home.udp)).is_err());
    home.start_again().await;
    app.home(|view| view.status == HomeStatus::Connected).await;
    app.mapping(PortMappingState::On).await;
    // 旧流向随旧连接没了；新的包开新流向。旧流向的结束和新包谁先到不一定，UDP 本来就可能丢包，所以重发。
    let reply = tokio::time::timeout(WAIT, async {
        loop {
            socket
                .send_to(b"after", (Ipv4Addr::LOCALHOST, home.udp))
                .await
                .unwrap();
            let mut buf = [0u8; 64];
            if let Ok(Ok(n)) =
                tokio::time::timeout(Duration::from_millis(500), socket.recv(&mut buf)).await
            {
                return buf[..n].to_vec();
            }
        }
    })
    .await
    .expect("no flow after the tunnel came back");
    assert_eq!(reply, b"after");
    assert_eq!(tcp_round_trip(home.tcp, b"tcp after").await, b"tcp after");
}

/// 插件页在屏幕上就映射，看不见就放掉；自动映射不改主人手动打开的开关。
#[tokio::test]
async fn showing_a_plugin_page_maps_its_ports_and_leaving_releases_them() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    app.mapping(PortMappingState::Off).await;
    // 列表里再放一个带网页但没声明端口的插件；隧道层只认 PLUGIN（等于“这台服务器上没装”）。
    let mut other = wonton_client::InstalledPlugin {
        ..running("app.other")
    };
    other.web_url = Some("http://app.other.wonton.localhost/".to_owned());
    home.core
        .set_lists(vec![vec![remote(home.tcp, home.udp), other]]);

    app.show(Some(PLUGIN)).await;
    let mapping = app.mapping(PortMappingState::On).await;
    assert!(!mapping.enabled, "自动映射不动手动开关");
    assert_eq!(tcp_round_trip(home.tcp, b"visible").await, b"visible");

    // 屏幕上换成了别的插件：PLUGIN 的端口全放掉。
    app.show(Some("app.other")).await;
    app.mapping(PortMappingState::Off).await;
    std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, home.tcp))).unwrap();
    let held = bind_udp_until_free(home.udp).await;
    assert!(!app.mapping(PortMappingState::Off).await.enabled);
    // 没声明端口的插件不多出条目。
    assert_eq!(app.core.plugins().port_mappings.len(), 1);
    drop(held);

    // 页面换回来，不用动开关又映射上。
    app.show(Some(PLUGIN)).await;
    app.mapping(PortMappingState::On).await;
    assert_eq!(tcp_round_trip(home.tcp, b"again").await, b"again");
}

/// widget 页（ADR-0096）在栈顶时也映射插件的端口：通话的媒体走这些端口；关掉页面就放掉。
#[tokio::test]
async fn showing_a_widget_page_maps_its_ports_and_closing_releases_them() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    app.mapping(PortMappingState::Off).await;
    let mut call = remote(home.tcp, home.udp);
    call.matrix_widget = Some(wonton_client::PluginWidget {
        url: "/room?widgetId=$matrix_widget_id".to_owned(),
        entry: wonton_client::WidgetEntry::VideoCall,
        capabilities: Vec::new(),
    });
    home.core.set_lists(vec![vec![call]]);
    // 列表不会自己重取（没有插件页开着就不轮询）：明说刷新一次。
    app.core
        .dispatch(wonton_client::Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    app.plugins(|view| {
        view.installed.as_ref().is_some_and(|list| {
            list.plugins
                .iter()
                .any(|plugin| plugin.matrix_widget.is_some())
        })
    })
    .await;

    // 没登录聊天，驱动起不来、页面停在出错上，但它在栈顶：映射看的是页面在不在，不是驱动。
    let widget = wonton_client::TabRef::Widget {
        plugin_id: PLUGIN.to_owned(),
        room_id: "!call:home.example".to_owned(),
    };
    let deadline = std::time::Instant::now() + WAIT;
    while let Err(error) = app.core.dispatch(wonton_client::Action::Pages {
        action: wonton_protocol::PageAction::OpenPage {
            tab: widget.clone(),
        },
    }) {
        // 页面那边可能还没收到新列表，同 `App::show`。
        assert!(
            std::time::Instant::now() < deadline,
            "打不开 widget 页：{error:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        app.core.pages().stack.last().map(|open| open.tab.clone()),
        Some(widget)
    );
    let mapping = app.mapping(PortMappingState::On).await;
    assert!(!mapping.enabled, "自动映射不动手动开关");
    assert_eq!(tcp_round_trip(home.tcp, b"call").await, b"call");

    app.core
        .dispatch(wonton_client::Action::Pages {
            action: wonton_protocol::PageAction::ClosePage,
        })
        .unwrap();
    app.mapping(PortMappingState::Off).await;
}

/// 手动打开的映射不被页面看不见关掉：再连一次，服务器拒绝的原因照样经还开着的监听报上来。
#[tokio::test]
async fn a_manually_opened_mapping_survives_the_page_becoming_invisible() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    app.mapping(PortMappingState::Off).await;
    app.map(true);
    app.mapping(PortMappingState::On).await;
    app.show(Some(PLUGIN)).await;
    app.show(None).await;

    home.refuse(Some(PortResolveError::PluginNotRunning));
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, home.tcp))
        .await
        .unwrap();
    tcp.write_all(b"x").await.unwrap();
    assert!(
        app.mapping(PortMappingState::PluginNotRunning)
            .await
            .enabled
    );
    home.refuse(None);
    assert!(
        std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, home.tcp))).is_err()
    );
    assert_eq!(tcp_round_trip(home.tcp, b"still").await, b"still");
}

/// App 进后台一律算不可见，回前台恢复，壳不用重报。
#[tokio::test]
async fn going_to_the_background_counts_as_invisible_and_foreground_restores_the_page() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    app.mapping(PortMappingState::Off).await;
    app.show(Some(PLUGIN)).await;
    app.mapping(PortMappingState::On).await;

    app.core
        .dispatch(wonton_client::Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    app.mapping(PortMappingState::Off).await;
    std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, home.tcp))).unwrap();
    let held = bind_udp_until_free(home.udp).await;
    drop(held);

    app.core
        .dispatch(wonton_client::Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
    app.mapping(PortMappingState::On).await;
    assert_eq!(tcp_round_trip(home.tcp, b"back").await, b"back");
}

/// 后台期间页面被换走：回前台不吃旧的可见性。
#[tokio::test]
async fn a_page_left_while_backgrounded_stays_invisible_on_foreground() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    app.mapping(PortMappingState::Off).await;
    app.show(Some(PLUGIN)).await;
    app.mapping(PortMappingState::On).await;

    app.core
        .dispatch(wonton_client::Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    app.mapping(PortMappingState::Off).await;
    app.show(None).await;
    app.core
        .dispatch(wonton_client::Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
    assert_eq!(
        app.core.plugins().port_mappings[0].state,
        PortMappingState::Off
    );
    app.show(Some(PLUGIN)).await;
    app.mapping(PortMappingState::On).await;
}

/// 页面开着时把开关关掉仍映射；开关只代表手动，页面看不见了才停。
#[tokio::test]
async fn turning_the_switch_off_while_the_page_is_visible_keeps_mapping() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    app.mapping(PortMappingState::Off).await;
    app.show(Some(PLUGIN)).await;
    app.mapping(PortMappingState::On).await;

    app.map(false);
    // 关掉开关后仍映射：再连一次，服务器拒绝的原因照样经还开着的监听报上来。
    home.refuse(Some(PortResolveError::PluginNotRunning));
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, home.tcp))
        .await
        .unwrap();
    tcp.write_all(b"x").await.unwrap();
    let mapping = app.mapping(PortMappingState::PluginNotRunning).await;
    assert!(!mapping.enabled, "页面可见时关开关只是去掉手动");
    home.refuse(None);
    assert_eq!(
        tcp_round_trip(home.tcp, b"still visible").await,
        b"still visible"
    );

    app.show(None).await;
    app.mapping(PortMappingState::Off).await;
    std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, home.tcp))).unwrap();
    bind_udp_until_free(home.udp).await;
}

/// 手动映射是给第三方 App 用的，App 进后台照常。
#[tokio::test]
async fn a_manually_opened_mapping_stays_on_in_the_background() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path());
    app.connect(&home);
    app.mapping(PortMappingState::Off).await;
    app.map(true);
    app.mapping(PortMappingState::On).await;

    app.core
        .dispatch(wonton_client::Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    // 拒绝同步：能等到拒绝原因，说明监听还在，后台那次核对也处理完了。
    home.refuse(Some(PortResolveError::PluginNotRunning));
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, home.tcp))
        .await
        .unwrap();
    tcp.write_all(b"x").await.unwrap();
    assert!(
        app.mapping(PortMappingState::PluginNotRunning)
            .await
            .enabled
    );
    home.refuse(None);
    assert!(
        std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, home.tcp))).is_err()
    );

    app.core
        .dispatch(wonton_client::Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
    assert_eq!(tcp_round_trip(home.tcp, b"back").await, b"back");
    assert!(app.mapping(PortMappingState::On).await.enabled);
}

#[tokio::test]
async fn turning_mapping_off_frees_the_ports_and_the_switch_is_remembered() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = mapped(&home, dir.path()).await;
    drop(app);
    let app = App::open(dir.path());
    app.connect(&home);
    assert!(
        app.mapping(PortMappingState::On).await.enabled,
        "switch kept per server"
    );
    app.map(false);
    assert!(!app.mapping(PortMappingState::Off).await.enabled);
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, home.tcp));
    std::net::TcpListener::bind(addr).unwrap();
    let _held = bind_udp_until_free(home.udp).await;
}

/// 服务器拒绝开端口时按原因显示，TCP 和 UDP 一样；下一次开成功就回到“开着”。
#[tokio::test]
async fn refusals_show_their_cause_until_the_next_open_succeeds() {
    let home = MapHome::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = mapped(&home, dir.path()).await;
    for (refusal, state) in [
        (
            PortResolveError::PluginNotInstalled,
            PortMappingState::PluginNotInstalled,
        ),
        (
            PortResolveError::PortNotDeclared,
            PortMappingState::PortNotDeclared,
        ),
        (
            PortResolveError::PluginNotRunning,
            PortMappingState::PluginNotRunning,
        ),
        (
            PortResolveError::UpstreamUnavailable,
            PortMappingState::PluginUnreachable,
        ),
    ] {
        home.refuse(Some(refusal));
        let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, home.tcp))
            .await
            .unwrap();
        tcp.write_all(b"x").await.unwrap();
        app.mapping(state).await;
        home.refuse(None);
        assert_eq!(tcp_round_trip(home.tcp, b"again").await, b"again");
        app.mapping(PortMappingState::On).await;
    }
    home.refuse(Some(PortResolveError::PluginNotRunning));
    local_udp()
        .await
        .send_to(b"x", (Ipv4Addr::LOCALHOST, home.udp))
        .await
        .unwrap();
    app.mapping(PortMappingState::PluginNotRunning).await;
    home.refuse(None);
    assert_eq!(exchange(&local_udp().await, home.udp, b"udp").await, b"udp");
    app.mapping(PortMappingState::On).await;
}
