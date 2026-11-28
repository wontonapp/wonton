//! 入口：按渲染出的配置起一条真 HAProxy，属主用证书文件的。

use std::os::unix::fs::MetadataExt;

use super::*;

pub(super) fn up(case: &mut Case, manager: &CertificateManager) -> Rendered {
    let entry = case.names.oneshot_container("entry");
    let certs = case.install.join("entry/certs");
    let mut model = Model::federation_only(ZONE, ZONE);
    model.public_labels = Vec::new();
    model.connectors = vec![case.connector_ip.clone()];
    let rendered = render(&model).unwrap();
    write(&certs.join("public.crt-list"), &rendered.public_crt_list);
    write(&certs.join("lan.crt-list"), &rendered.lan_crt_list);
    let entry_config = case.dir.path().join("entry.cfg");
    write(&entry_config, &rendered.config);
    let site = manager.certificate_path();
    let owner = format!(
        "{}:{}",
        std::fs::metadata(&site).unwrap().uid(),
        std::fs::metadata(&site).unwrap().gid()
    );
    start(
        &case.names,
        &case.network,
        &entry,
        Some(&owner),
        &case.entry_ip,
        &["entry-edge.wonton.internal", "entry.wonton.internal"],
        &[
            (certs, "/certs".into()),
            (entry_config, "/config/haproxy.cfg".into()),
        ],
        &case.haproxy,
        "haproxy",
        // 和产品一样开主进程管理端口（-S）：热加载走它，expose-fd 靠它把监听套接字交给新进程。
        &[
            "-W",
            "-db",
            "-f",
            "/config/haproxy.cfg",
            "-S",
            &format!("{}:8404", case.entry_ip),
        ],
    );
    case.entry = entry;
    rendered
}
