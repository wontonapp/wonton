//! 假曲库的 HTTP：`/rest/<方法>` 按 OpenSubsonic 回 JSON 或图片，`/app/` 是代登录建号那一下，别的 404。

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::CONTENT_TYPE;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};

use super::catalog::{Catalog, Reply};

pub(super) async fn serve(listener: std::net::TcpListener) {
    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
        tracing::warn!("the fake music library could not listen");
        return;
    };
    let catalog = Arc::new(Catalog::new());
    while let Ok((stream, _)) = listener.accept().await {
        let catalog = catalog.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let catalog = catalog.clone();
                async move { Ok::<_, Infallible>(respond(&catalog, &request)) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

fn respond(catalog: &Catalog, request: &Request<Incoming>) -> Response<Full<Bytes>> {
    let path = request.uri().path();
    if path == "/app/" {
        return reply(StatusCode::OK, "text/html", Bytes::from_static(b"ok"));
    }
    let Some(method) = path.strip_prefix("/rest/") else {
        return reply(StatusCode::NOT_FOUND, "text/plain", Bytes::new());
    };
    let params: BTreeMap<String, String> =
        url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let body = match catalog.respond(method, &params) {
        Reply::Image(bytes) => return reply(StatusCode::OK, "image/bmp", bytes.into()),
        Reply::Ok(mut fields) => {
            fields.insert("status".into(), "ok".into());
            fields.insert("version".into(), "1.16.1".into());
            Value::Object(fields)
        }
        Reply::Failed(code, message) => json!({
            "status": "failed",
            "version": "1.16.1",
            "error": {"code": code, "message": message},
        }),
    };
    let body = json!({ "subsonic-response": body }).to_string();
    reply(StatusCode::OK, "application/json", body.into())
}

fn reply(status: StatusCode, kind: &'static str, body: Bytes) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, hyper::header::HeaderValue::from_static(kind));
    response
}
