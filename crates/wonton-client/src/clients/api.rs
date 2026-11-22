//! `/_wonton/clients/domain` 的请求：经本地代理，带 `X-Wonton-Clients: 1`。
//!
//! 失败的类别读 `X-Wonton-Error` 响应头，说明读错误体的 `message`；本地代理自己回的错误没有这个头，按状态码分。

use std::time::Duration;
use reqwest::Client;
use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use serde::de::DeserializeOwned;
use serde_json::json;
use url::Url;
