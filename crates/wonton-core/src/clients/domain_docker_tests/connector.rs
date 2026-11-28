//! 连接器：PROXY v2 把客户端的真实来源交给入口。

use super::*;

pub(super) fn up(case: &mut Case) {
    let connector = case.names.oneshot_container("connector");
    let connector_config = case.dir.path().join("connector.cfg");
    write(
        &connector_config,
        "global\ndefaults\n  mode tcp\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\nresolvers docker\n  nameserver docker 127.0.0.11:53\n  resolve_retries 1\n  timeout resolve 1s\n  timeout retry 1s\n  hold valid 10s\nfrontend c\n  bind :8443\n  default_backend entry\nbackend entry\n  server entry entry-edge.wonton.internal:8443 resolvers docker init-addr libc,none send-proxy-v2\n",
    );
    start(
        &case.names,
        &case.network,
        &connector,
        None,
        &case.connector_ip,
        &["connector.wonton.internal"],
        &[(connector_config, "/config/connector.cfg".into())],
        &case.haproxy,
        "haproxy",
        &["-W", "-db", "-f", "/config/connector.cfg"],
    );
    case.connector = connector;
}
