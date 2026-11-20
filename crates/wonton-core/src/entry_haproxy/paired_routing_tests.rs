//! 8449 配对前端只把核心自己答的路径交给 8484，其余直给 Tuwunel。
//!
//! 从隧道进来的只有已配对的云吞，`Host` 已被校验成本机服务器名（不改写）；客户端 API（`/_matrix/client/*`）在
//! 8484 上是 404，必须由 Tuwunel 自己答。联邦、key 和三个 well-known 仍走核心：前者要核心的转发与错误类别，
//! 后者由核心生成。

use super::model::Model;
use super::render::render;

fn paired(config: &str) -> &str {
    let start = config.find("frontend paired\n").expect("没有配对前端");
    let end = config[start + 1..]
        .find("\nfrontend ")
        .map_or(config.len(), |offset| start + 1 + offset);
    &config[start..end]
}

/// `use_backend gateway` 行里列的条件；每行一个表达式。
fn gateway_rules(config: &str) -> Vec<&str> {
    paired(config)
        .lines()
        .filter(|line| line.trim_start().starts_with("use_backend gateway"))
        .collect()
}

#[test]
fn client_api_paths_go_straight_to_tuwunel() {
    let rendered = render(&Model::federation_only("home.example", "home.example")).unwrap();
    let frontend = paired(&rendered.config);
    assert!(frontend.contains("default_backend tuwunel"));
    assert!(
        !frontend.contains("/_matrix/client"),
        "客户端 API 不该出现在任何网关条件里：{frontend}"
    );
    // `/_wonton/*`（App 的接口）只经隧道到主网关，配对前端上没有它的入口；落 Tuwunel 回 404。
    assert!(
        !frontend.contains("/_wonton/"),
        "`/_wonton/*` 不该进网关条件：{frontend}"
    );
    assert!(
        rendered
            .config
            .contains("\nbackend tuwunel\n  server tuwunel tuwunel.wonton.internal:8008"),
        "{}",
        rendered.config
    );
}

#[test]
fn federation_and_key_requests_go_to_the_gateway() {
    let rendered = render(&Model::federation_only("home.example", "home.example")).unwrap();
    let rules = gateway_rules(&rendered.config).join("\n");
    for path in ["/_matrix/federation/", "/_matrix/key/"] {
        assert!(rules.contains(&format!("path_beg {path}")), "{rules}");
    }
}

#[test]
fn discovery_files_stay_with_the_gateway() {
    let rendered = render(&Model::federation_only("home.example", "home.example")).unwrap();
    let rules = gateway_rules(&rendered.config).join("\n");
    for path in [
        "/.well-known/matrix/server",
        "/.well-known/matrix/client",
        "/.well-known/wonton",
    ] {
        assert!(rules.contains(&format!("path {path}")), "{rules}");
    }
}

/// `use_backend` 在请求阶段才求值：Host 不对的 421 必须先写、先命中，不能被分流绕过去。
#[test]
fn host_mismatch_is_rejected_before_any_backend_switch() {
    let rendered = render(&Model::federation_only("home.example", "home.example")).unwrap();
    let frontend = paired(&rendered.config);
    let sni = frontend.find("unless correct_sni").expect("没有 SNI 检查");
    let host = frontend.find("hdr(host)").expect("没有 Host 检查");
    let switch = frontend.find("use_backend").expect("没有分流");
    assert!(sni < host && host < switch, "{frontend}");
}

/// 前端只盖入口标记、不改 `Host`；核心在转发联邦时按这个标记认出配对来的请求。Tuwunel 不看 `Host`。
#[test]
fn paired_frontend_marks_the_entry_and_keeps_the_host() {
    let rendered = render(&Model::federation_only("home.example", "home.example")).unwrap();
    let frontend = paired(&rendered.config);
    assert!(frontend.contains("http-request set-header X-Wonton-Entry paired"));
    assert!(!frontend.contains("http-request set-header Host"));
}
