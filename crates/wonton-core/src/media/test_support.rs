//! 媒体测试共用的假目录和假提供方外壳：目录记下签发与收回，外壳记下每个请求的路径、凭据和替谁。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::Request;
use axum::http::header;
use axum::middleware::{self, Next};
use axum::routing::get;
use serde_json::json;
use tokio::net::TcpListener;
use wonton_protocol::media::{MediaKind, MediaOperation, WONTON_USER_HEADER};

use super::provider::{Candidate, Pending, ProviderDirectory};
use crate::plugin::upstreams::Endpoint;

#[derive(Default, Clone)]
pub(crate) struct Book {
    pub issued: Arc<Mutex<Vec<(String, String, String)>>>,
    pub revoked: Arc<Mutex<Vec<(String, String)>>>,
}

pub(crate) struct FakeDirectory {
    pub candidates: Vec<Candidate>,
    pub endpoints: HashMap<String, Endpoint>,
    pub book: Book,
}

impl FakeDirectory {
    pub(crate) fn new(entries: Vec<(Candidate, Endpoint)>) -> (Self, Book) {
        let book = Book::default();
        let directory = Self {
            candidates: entries.iter().map(|(c, _)| c.clone()).collect(),
            endpoints: entries
                .into_iter()
                .map(|(c, endpoint)| (c.id, endpoint))
                .collect(),
            book: book.clone(),
        };
        (directory, book)
    }
}

impl ProviderDirectory for FakeDirectory {
    fn candidates(&self) -> Vec<Candidate> {
        self.candidates.clone()
    }

    fn endpoint<'a>(&'a self, provider: &'a str) -> Pending<'a, Endpoint> {
        let endpoint = self.endpoints[provider].clone();
        Box::pin(async move { Ok(endpoint) })
    }

    fn issue<'a>(&'a self, provider: &'a str, name: &'a str, key: &'a str) -> Pending<'a, ()> {
        self.book
            .issued
            .lock()
            .unwrap()
            .push((provider.into(), name.into(), key.into()));
        Box::pin(async { Ok(()) })
    }

    fn revoke<'a>(&'a self, provider: &'a str, name: &'a str) -> Pending<'a, ()> {
        self.book
            .revoked
            .lock()
            .unwrap()
            .push((provider.into(), name.into()));
        Box::pin(async { Ok(()) })
    }
}

/// 假提供方收到的请求：方法与路径（带查询串）、凭据、替谁。
pub(crate) type Seen = Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>;

pub(crate) struct Served(tokio::task::JoinHandle<()>);

impl Drop for Served {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) async fn serve(router: Router) -> (Endpoint, Seen, Served) {
    let seen = Seen::default();
    let record = seen.clone();
    let router = router.layer(middleware::from_fn(move |req: Request, next: Next| {
        let record = record.clone();
        async move {
            let entry = {
                let header = |name: &str| {
                    req.headers()
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned)
                };
                let target = req
                    .uri()
                    .path_and_query()
                    .map_or_else(String::new, |pq| pq.as_str().to_owned());
                (
                    format!("{} {target}", req.method()),
                    header(header::AUTHORIZATION.as_str()),
                    header(WONTON_USER_HEADER),
                )
            };
            record.lock().unwrap().push(entry);
            next.run(req).await
        }
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (endpoint, seen, Served(task))
}

/// 只会自报能力的假提供方，其余路由由调用方接着加。
pub(crate) fn caps(operations: &[&str], kinds: &[&str]) -> Router {
    let body = json!({"operations": operations, "kinds": kinds});
    Router::new().route(
        "/_wonton/capabilities",
        get(move || {
            let body = body.clone();
            async move { axum::Json(body) }
        }),
    )
}

pub(crate) fn candidate(id: &str, operations: &[MediaOperation], kinds: &[MediaKind]) -> Candidate {
    Candidate {
        id: id.into(),
        operations: operations.to_vec(),
        kinds: kinds.to_vec(),
        default: false,
    }
}
