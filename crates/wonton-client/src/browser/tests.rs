use std::net::Ipv4Addr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use wonton_protocol::{
    HomeStatus, InstalledPlugins, PluginHealth, PluginKind, PluginState, PluginTrust, ProxySettings,
};

use super::*;

const BUDGET: &str = "http://org-actualbudget-actual.wonton.localhost:8480";
const PHOTOS: &str = "http://app-photos.wonton.localhost:8480";

fn plugin(id: &str, origin: &str, isolated: bool) -> InstalledPlugin {
    InstalledPlugin {
        chat: None,
        id: id.to_owned(),
        title: id.to_owned(),
        summary: String::new(),
        version: "1.0.0".to_owned(),
        revision: "1".to_owned(),
        kind: PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status: PluginState::Running {
            health: PluginHealth::Healthy,
        },
        web_url: Some(format!("{origin}/")),
        link_url: None,
        cross_origin_isolated: isolated,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: None,
        family: false,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        install_failure: None,
        tunnel_ports: Vec::new(),
    }
}

fn installed(plugins: Vec<InstalledPlugin>) -> PluginsViewModel {
    PluginsViewModel {
        installed: Some(InstalledPlugins {
            plugins,
            upgrade_check: None,
        }),
        ..Default::default()
    }
}

fn home(server: Option<&str>, proxy: Option<ProxySettings>) -> HomeViewModel {
    HomeViewModel {
        device_id: "device".to_owned(),
        server_id: server.map(str::to_owned),
        status: HomeStatus::Connected,
        path: None,
        notice: None,
        details: Default::default(),
        proxy,
        web_url: None,
        pairing: None,
        invite: None,
        keep_connection: false,
        just_recovered: false,
    }
}

fn proxy_at(addr: SocketAddr, credential: &ProxyCredential) -> ProxySettings {
    ProxySettings {
        host: addr.ip().to_string(),
        port: addr.port(),
        username: credential.username().to_owned(),
        password: credential.password().to_owned(),
        domain: "wonton.localhost".to_owned(),
    }
}

struct Rig {
    link: Arc<BrowserLink>,
    home: watch::Sender<HomeViewModel>,
    plugins: watch::Sender<PluginsViewModel>,
}

fn rig(key: u8, server: Option<&str>) -> Rig {
    let home = watch::Sender::new(home(server, None));
    let plugins = watch::Sender::new(installed(vec![
        plugin("org.actualbudget.actual", BUDGET, true),
        plugin("app.photos", PHOTOS, false),
        plugin(
            "app.other-isolated",
            "http://app-other-isolated.wonton.localhost:8480",
            true,
        ),
    ]));
    let link = BrowserLink::new(
        Handle::current(),
        [key; 32],
        home.subscribe(),
        plugins.subscribe(),
    );
    Rig {
        link,
        home,
        plugins,
    }
}

/// `http://<标签>.localhost:<端口><其余>` 拆成（标签，端口，其余）。
fn split(url: &str) -> (String, u16, String) {
    let rest = url.strip_prefix("http://").unwrap();
    let (label, rest) = rest.split_once(".localhost:").unwrap();
    let at = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    (
        label.to_owned(),
        rest[..at].parse().unwrap(),
        rest[at..].to_owned(),
    )
}

#[test]
fn only_isolated_pages_on_android_webview_go_to_the_browser() {
    let isolated = plugin("org.actualbudget.actual", BUDGET, true);
    let plain = plugin("app.photos", PHOTOS, false);
    let mut no_page = isolated.clone();
    no_page.web_url = None;
    assert!(opens_in_browser(&isolated, WebEngine::AndroidWebView));
    assert!(!opens_in_browser(&isolated, WebEngine::WebKit));
    assert!(!opens_in_browser(&plain, WebEngine::AndroidWebView));
    assert!(!opens_in_browser(&no_page, WebEngine::AndroidWebView));
    assert_eq!(
        ENGINE == WebEngine::AndroidWebView,
        cfg!(target_os = "android")
    );
}

#[tokio::test]
async fn an_isolated_page_gets_a_stable_secret_address_on_loopback() {
    let rig = rig(1, Some("server-a"));
    let url = rig
        .link
        .open(&format!("{BUDGET}/budget/accounts?month=11#top"))
        .unwrap();
    let (label, port, rest) = split(&url);
    assert_eq!(label.len(), LABEL_LEN);
    assert!(
        label
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    assert_eq!(rest, "/budget/accounts?month=11#top");
    assert!((PORT_BASE..PORT_BASE + PORT_SPAN).contains(&preferred_port(&[1; 32])));

    // 同一个页面每次都是同一个 origin：浏览器里存的数据还在。
    let again = split(&rig.link.open(&format!("{BUDGET}/")).unwrap());
    assert_eq!((again.0, again.1), (label.clone(), port));
    // 别的插件、别的服务器、别的设备，标签都不同。
    let other = rig
        .link
        .open("http://app-other-isolated.wonton.localhost:8480/")
        .unwrap();
    assert_ne!(split(&other).0, label);
    let other_server = super::label(&[1; 32], "server-b", "org.actualbudget.actual");
    let other_device = super::label(&[2; 32], "server-a", "org.actualbudget.actual");
    assert_ne!(other_server, label);
    assert_ne!(other_device, label);
    assert_eq!(
        super::label(&[1; 32], "server-a", "org.actualbudget.actual"),
        label
    );
}

#[tokio::test]
async fn pages_that_work_in_the_app_are_not_handed_out() {
    let rig = rig(1, Some("server-a"));
    for url in [
        format!("{PHOTOS}/"),
        "http://unknown.wonton.localhost:8480/".to_owned(),
        "https://example.com/".to_owned(),
        "not a url".to_owned(),
        // 端口不同就是别的 origin。
        "http://org-actualbudget-actual.wonton.localhost:9999/".to_owned(),
    ] {
        let error = rig.link.open(&url).unwrap_err();
        assert!(
            matches!(error, CoreError::InvalidInput { .. }),
            "{url}: {error:?}"
        );
    }
    let unpaired = self::rig(1, None);
    let error = unpaired.link.open(&format!("{BUDGET}/")).unwrap_err();
    assert!(matches!(error, CoreError::NotReady { .. }), "{error:?}");
}

/// 假的本地代理：收一个 `CONNECT`，回 200，再收下转来的请求头，回 `ok`。
async fn fake_proxy() -> (SocketAddr, ProxyCredential, oneshot::Receiver<String>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (sender, seen) = oneshot::channel();
    tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        read_head(&mut tcp).await;
        tcp.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let request = read_head(&mut tcp).await;
        tcp.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        sender.send(request).ok();
    });
    (addr, ProxyCredential::generate(), seen)
}

async fn read_head(tcp: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        tcp.read_exact(&mut byte).await.unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}

/// 像浏览器一样访问 `url`（主机名自己解析到回环），读完整个响应。
async fn browse(url: &str) -> String {
    let (label, port, rest) = split(url);
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let request = format!("GET {rest} HTTP/1.1\r\nHost: {label}.localhost:{port}\r\n\r\n");
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    tcp.read_to_string(&mut response).await.unwrap();
    response
}

#[tokio::test]
async fn the_browser_reaches_the_plugin_through_the_apps_proxy() {
    let rig = rig(3, Some("server-a"));
    let (addr, credential, seen) = fake_proxy().await;
    rig.home
        .send_replace(home(Some("server-a"), Some(proxy_at(addr, &credential))));
    let url = rig.link.open(&format!("{BUDGET}/budget")).unwrap();

    let response = browse(&url).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    let forwarded = seen.await.unwrap();
    assert!(
        forwarded.starts_with("GET /budget HTTP/1.1\r\n"),
        "{forwarded}"
    );
    assert!(forwarded.contains("\r\nHost: org-actualbudget-actual.wonton.localhost:8480\r\n"));

    // 断开后浏览器得到一句人话，不是挂住。
    rig.home.send_replace(home(Some("server-a"), None));
    let response = browse(&url).await;
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
    assert!(
        response.contains("x-wonton-tunnel-error: transport-unavailable\r\n"),
        "{response}"
    );

    // 插件卸载了，地址就失效。
    rig.home
        .send_replace(home(Some("server-a"), Some(proxy_at(addr, &credential))));
    rig.plugins.send_replace(installed(Vec::new()));
    let response = browse(&url).await;
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");
    assert!(
        response.contains("x-wonton-tunnel-error: unknown-address\r\n"),
        "{response}"
    );
}
