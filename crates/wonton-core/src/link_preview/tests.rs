//! 链接预览的行为测试：假 DNS 和拨号把“公网”接到本机的假站点（[`super::test_support`]），断言类别和看得到的效果。

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use wonton_protocol::LinkPreviewFailure;

use super::test_support::{SITE_IP, og_page, png, world, world_with};
use super::{Limits, LinkPreviews};

fn html(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

async fn failure(previews: &LinkPreviews, url: &str) -> LinkPreviewFailure {
    match previews.preview(url, "device").await {
        Ok(preview) => panic!("{url} 应该取不到，却得到 {preview:?}"),
        Err(failure) => failure.kind,
    }
}

#[tokio::test]
async fn a_page_gives_its_card_and_the_image_is_kept_for_the_device() {
    let routes = Router::new()
        .route(
            "/post",
            get(|| async { html(og_page("Hello", Some("/img.png"))) }),
        )
        .route(
            "/img.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], png(4, 3)) }),
        );
    let (previews, world) = world(routes).await;
    let preview = previews
        .preview("http://site.test/post#top", "device")
        .await
        .unwrap();
    assert_eq!(preview.url, "http://site.test/post");
    assert_eq!(preview.title.as_deref(), Some("Hello"));
    assert_eq!(preview.description.as_deref(), Some("About Hello"));
    assert_eq!(preview.site_name.as_deref(), Some("Site"));
    let image = preview.image.expect("应该带图片");
    assert_eq!(image.mime_type, "image/png");
    assert_eq!((image.width, image.height), (Some(4), Some(3)));
    let cached = previews.image(&image.id).expect("图片应该在缓存里");
    assert_eq!(cached.bytes, png(4, 3));
    assert_eq!(cached.mime_type, "image/png");
    // 连的是检查过的 IP，`Host` 仍是原域名。
    let site_ip: std::net::IpAddr = SITE_IP.parse().unwrap();
    assert!(
        world
            .dialed
            .lock()
            .unwrap()
            .iter()
            .all(|a| a.ip() == site_ip)
    );
    assert!(
        world
            .hits
            .lock()
            .unwrap()
            .iter()
            .all(|hit| hit.starts_with("site.test "))
    );
}

#[tokio::test]
async fn addresses_inside_the_home_are_refused_before_connecting() {
    let dns: &[(&str, &[&str])] = &[
        ("loopback.test", &["127.0.0.1"]),
        ("mixed.test", &[SITE_IP, "10.0.0.7"]),
        ("ula.test", &["fd00::1"]),
        ("mapped.test", &["::ffff:192.168.1.1"]),
    ];
    let routes = Router::new().route("/", get(|| async { html(og_page("x", None)) }));
    let (previews, world) = world_with(routes, dns, Limits::default(), None).await;
    for url in [
        "http://loopback.test/",
        "http://mixed.test/",
        "http://ula.test/",
        "http://mapped.test/",
        "http://127.0.0.1/",
        "http://[::1]/",
        "http://169.254.169.254/latest/meta-data/",
        "http://100.64.0.1/",
        "http://[::ffff:10.0.0.1]/",
        "http://0.0.0.0/",
        "http://224.0.0.1/",
    ] {
        assert_eq!(
            failure(&previews, url).await,
            LinkPreviewFailure::BlockedAddress,
            "{url}"
        );
    }
    assert!(world.dialed.lock().unwrap().is_empty(), "一个都不该去连");
    assert_eq!(world.hit_count(), 0);
}

#[tokio::test]
async fn a_redirect_into_the_home_is_refused_at_that_hop() {
    let routes = Router::new()
        .route(
            "/away",
            get(|| async { Redirect::temporary("http://inside.test/admin") }),
        )
        .route(
            "/literal",
            get(|| async { Redirect::permanent("http://192.168.1.1/") }),
        );
    let dns: &[(&str, &[&str])] = &[("inside.test", &["192.168.1.1"])];
    let (previews, world) = world_with(routes, dns, Limits::default(), None).await;
    for url in ["http://site.test/away", "http://site.test/literal"] {
        assert_eq!(
            failure(&previews, url).await,
            LinkPreviewFailure::BlockedAddress
        );
    }
    let private: std::net::IpAddr = "192.168.1.1".parse().unwrap();
    assert!(
        !world
            .dialed
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.ip() == private)
    );
    assert_eq!(world.hit_count(), 2);
}

#[tokio::test]
async fn only_http_and_the_four_web_ports_are_fetched() {
    let routes = Router::new()
        .route("/", get(|| async { html(og_page("Port", None)) }))
        .route(
            "/to-ssh",
            get(|| async { Redirect::temporary("http://site.test:22/") }),
        );
    let (previews, world) = world(routes).await;
    for url in [
        "ftp://site.test/",
        "file:///etc/passwd",
        "http://site.test:8081/",
        "http://site.test:22/",
        "https://site.test:6443/",
        "http://user:secret@site.test/",
        "not a url",
        "http://site.test/to-ssh",
    ] {
        assert_eq!(
            failure(&previews, url).await,
            LinkPreviewFailure::InvalidUrl,
            "{url}"
        );
    }
    assert_eq!(world.hits_of("/"), 0);
    let preview = previews
        .preview("http://site.test:8080/", "device")
        .await
        .unwrap();
    assert_eq!(preview.title.as_deref(), Some("Port"));
}

#[tokio::test]
async fn three_redirects_are_followed_and_a_fourth_is_an_upstream_error() {
    let routes = Router::new().route(
        "/r/{n}",
        get(
            |axum::extract::Path(n): axum::extract::Path<u32>| async move {
                if n == 0 {
                    html(og_page("Landed", None))
                } else {
                    Redirect::temporary(&format!("/r/{}", n - 1)).into_response()
                }
            },
        ),
    );
    let (previews, _world) = world(routes).await;
    let preview = previews
        .preview("http://site.test/r/3", "device")
        .await
        .unwrap();
    assert_eq!(preview.url, "http://site.test/r/0");
    assert_eq!(
        failure(&previews, "http://site.test/r/4").await,
        LinkPreviewFailure::UpstreamError
    );
}

#[tokio::test]
async fn only_the_first_512_kib_of_a_page_are_read() {
    let padding = "x".repeat(600 << 10);
    let late = format!(
        r#"<html><head><!-- {padding} --><meta property="og:title" content="Late"></head></html>"#
    );
    let early = format!(
        r#"<html><head><meta property="og:title" content="Early"><!-- {padding} --></head></html>"#
    );
    let routes = Router::new()
        .route("/late", get(move || async move { html(late) }))
        .route("/early", get(move || async move { html(early) }));
    let (previews, _world) = world(routes).await;
    assert_eq!(
        failure(&previews, "http://site.test/late").await,
        LinkPreviewFailure::NoPreview
    );
    let preview = previews
        .preview("http://site.test/early", "device")
        .await
        .unwrap();
    assert_eq!(preview.title.as_deref(), Some("Early"));
}

/// 6 MiB 的“PNG”：开头是 PNG 签名，`chunked` 时不声明长度。
fn big_png(chunked: bool) -> Response {
    let mut bytes = png(1, 1);
    bytes.resize(6 << 20, 0);
    let body = if chunked {
        let chunks = bytes
            .chunks(64 << 10)
            .map(|c| Ok::<_, std::io::Error>(c.to_vec()));
        Body::from_stream(futures_util::stream::iter(chunks.collect::<Vec<_>>()))
    } else {
        Body::from(bytes)
    };
    ([(header::CONTENT_TYPE, "image/png")], body).into_response()
}

#[tokio::test]
async fn images_over_5_mib_or_of_other_types_are_left_off_the_card() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
    let routes = Router::new()
        .route("/big", get(|| async { big_png(false) }))
        .route("/streamed", get(|| async { big_png(true) }))
        .route(
            "/svg",
            get(move || async move { ([(header::CONTENT_TYPE, "image/png")], svg) }),
        )
        .route(
            "/p/{name}",
            get(
                |axum::extract::Path(name): axum::extract::Path<String>| async move {
                    html(og_page("Card", Some(&format!("/{name}"))))
                },
            ),
        )
        .route(
            "/only/{name}",
            get(
                |axum::extract::Path(name): axum::extract::Path<String>| async move {
                    html(format!(
                        r#"<head><meta property="og:image" content="/{name}"></head>"#
                    ))
                },
            ),
        );
    let (previews, _world) = world(routes).await;
    for name in ["big", "streamed", "svg"] {
        let preview = previews
            .preview(&format!("http://site.test/p/{name}"), "device")
            .await
            .unwrap();
        assert_eq!(preview.title.as_deref(), Some("Card"), "{name}");
        assert_eq!(preview.image, None, "{name}");
    }
    // 只有图片的网页，图片取不到就是取不到预览。
    assert_eq!(
        failure(&previews, "http://site.test/only/big").await,
        LinkPreviewFailure::UpstreamError
    );
}

#[tokio::test]
async fn things_that_are_not_pages_or_have_nothing_to_show_are_refused_by_kind() {
    let routes = Router::new()
        .route(
            "/api",
            get(|| async { ([(header::CONTENT_TYPE, "application/json")], "{}") }),
        )
        .route(
            "/photo.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], png(1, 1)) }),
        )
        .route(
            "/bare",
            get(|| async { html("<html><head></head><body>hi</body></html>".into()) }),
        )
        .route("/down", get(|| async { StatusCode::SERVICE_UNAVAILABLE }));
    let (previews, _world) = world(routes).await;
    for (path, kind) in [
        ("/api", LinkPreviewFailure::NotHtml),
        ("/photo.png", LinkPreviewFailure::NotHtml),
        ("/bare", LinkPreviewFailure::NoPreview),
        ("/down", LinkPreviewFailure::UpstreamError),
    ] {
        let url = format!("http://site.test{path}");
        assert_eq!(failure(&previews, &url).await, kind, "{path}");
    }
    assert_eq!(
        failure(&previews, "http://nowhere.test/").await,
        LinkPreviewFailure::UpstreamError
    );
}

#[tokio::test]
async fn a_slow_page_or_a_slow_image_times_out_the_whole_request() {
    let slow = || async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        html(og_page("Slow", None))
    };
    let routes = Router::new().route("/slow", get(slow)).route(
        "/slow-image-page",
        get(|| async { html(og_page("Fast", Some("/slow"))) }),
    );
    let limits = Limits {
        timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let (previews, _world) = world_with(routes, &[], limits, None).await;
    for path in ["/slow", "/slow-image-page"] {
        let url = format!("http://site.test{path}");
        let started = std::time::Instant::now();
        assert_eq!(failure(&previews, &url).await, LinkPreviewFailure::Timeout);
        assert!(started.elapsed() < Duration::from_secs(3), "{path}");
    }
}

#[tokio::test]
async fn each_device_is_limited_per_minute_but_cached_answers_are_free() {
    let routes = Router::new().route(
        "/{n}",
        get(
            |axum::extract::Path(n): axum::extract::Path<u32>| async move {
                html(og_page(&format!("Page {n}"), None))
            },
        ),
    );
    let limits = Limits {
        per_minute: 2,
        ..Limits::default()
    };
    let (previews, world) = world_with(routes, &[], limits, None).await;
    let url = |n: u32| format!("http://site.test/{n}");
    assert!(previews.preview(&url(1), "phone").await.is_ok());
    assert!(previews.preview(&url(2), "phone").await.is_ok());
    let third = previews.preview(&url(3), "phone").await.unwrap_err();
    assert_eq!(third.kind, LinkPreviewFailure::TooManyRequests);
    assert!(
        previews.preview(&url(1), "phone").await.is_ok(),
        "缓存不计数"
    );
    assert!(
        previews.preview(&url(3), "tablet").await.is_ok(),
        "按设备分开算"
    );
    world.advance(Duration::from_secs(61));
    assert!(previews.preview(&url(4), "phone").await.is_ok());
    assert_eq!(world.hits_of("/3"), 1);
}

#[tokio::test]
async fn a_preview_is_cached_for_a_day_with_its_image() {
    let routes = Router::new()
        .route(
            "/post",
            get(|| async { html(og_page("Cached", Some("/img.png"))) }),
        )
        .route(
            "/img.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], png(2, 2)) }),
        );
    let (previews, world) = world(routes).await;
    let first = previews
        .preview("http://site.test/post", "device")
        .await
        .unwrap();
    world.advance(Duration::from_secs(23 * 3600));
    let second = previews
        .preview("http://site.test/post", "device")
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!((world.hits_of("/post"), world.hits_of("/img.png")), (1, 1));
    let id = first.image.unwrap().id;
    assert!(previews.image(&id).is_some());

    world.advance(Duration::from_secs(3600));
    assert!(previews.image(&id).is_none(), "过期的图片不再给");
    previews
        .preview("http://site.test/post", "device")
        .await
        .unwrap();
    assert_eq!(world.hits_of("/post"), 2);
    assert!(previews.image(&id).is_some());
    assert!(previews.image("../../core/owner-token").is_none());
}

#[tokio::test]
async fn failures_are_cached_for_ten_minutes() {
    let routes = Router::new().route("/down", get(|| async { StatusCode::BAD_GATEWAY }));
    let (previews, world) = world(routes).await;
    let url = "http://site.test/down";
    assert_eq!(
        failure(&previews, url).await,
        LinkPreviewFailure::UpstreamError
    );
    world.advance(Duration::from_secs(9 * 60));
    assert_eq!(
        failure(&previews, url).await,
        LinkPreviewFailure::UpstreamError
    );
    assert_eq!(world.hits_of("/down"), 1);
    world.advance(Duration::from_secs(60));
    assert_eq!(
        failure(&previews, url).await,
        LinkPreviewFailure::UpstreamError
    );
    assert_eq!(world.hits_of("/down"), 2);
}
