//! 夹具页面：闲置连接不挡后面的请求、整页那张两种 viewport 声明都发。

use super::*;

#[test]
fn fixture_page_is_served_while_an_idle_connection_is_open() {
    let (_, page) = corner().expect("the loopback fixture page did not start");
    let url = page.url.unwrap();
    let (address, _) = url.trim_start_matches("http://").split_once('/').unwrap();
    // WebView 会先连一条不发请求的连接，后面的正式请求不能等它。
    let _idle = std::net::TcpStream::connect(address).unwrap();
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(b"GET /corner HTTP/1.1\r\nhost: fixture\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
}

#[test]
fn corner_page_is_served_with_and_without_viewport_fit() {
    let (tab, page) = corner().expect("the loopback fixture page did not start");
    let url = page.url.clone().unwrap();
    assert_eq!(tab, TabRef::Url { url: url.clone() });
    let (address, _) = url.trim_start_matches("http://").split_once('/').unwrap();
    let get = |path: &str| {
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        // 一次写完：服务只读一次就回，分几次写时没读走的那截会让连接被重置。
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nhost: fixture\r\n\r\n").as_bytes())
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let plain = get("/corner");
    assert!(plain.contains("Fixture corner") && !plain.contains("viewport-fit=cover"));
    assert!(get("/corner-cover").contains("viewport-fit=cover"));
}
