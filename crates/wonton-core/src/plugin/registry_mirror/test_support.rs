use std::os::unix::fs::MetadataExt;

use crate::config::{DEFAULT_INSTALL_TIMEOUT, PublicUrls};
use crate::http::HttpClient;
use crate::owner::{Owner, OwnerStore};
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::names::Instance;
use crate::plugin::store::Store;
use crate::plugin::storefront::StoreConfig;

pub(super) fn manager(engine: Engine, dir: &tempfile::TempDir, instance: &str) -> PluginManager {
    let owner = std::fs::metadata(dir.path()).unwrap();
    let cfg = LifecycleConfig {
        public: PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.localhost".into(),
            port: 8080,
        },
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: owner.uid(),
        pgid: owner.gid(),
        // 期限从建容器之前算起（lifecycle 的部署），满载时 Docker 建、启动容器本身就要十几到二十几秒，
        // 之后还要稳定运行 3 秒才算就绪，30 秒不够用；这两个测试验镜像缓存和摘要，不验安装速度，跟生产默认一致。
        install_timeout: DEFAULT_INSTALL_TIMEOUT,
        owner: "owner".into(),
        profile: OwnerStore::for_test(
            dir.path(),
            &Owner {
                username: "owner".into(),
                display_name: "Owner".into(),
                email: None,
            },
        ),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Instance::parse(Some(instance)).unwrap(),
    };
    let index_dir = dir.path().join("index");
    std::fs::create_dir(&index_dir).unwrap();
    std::fs::write(index_dir.join("index.json"), r#"{"format":1}"#).unwrap();
    PluginManager::new(
        engine,
        Store::open(dir.path().join("plugins")).unwrap(),
        HttpClient::new(),
        None,
        cfg,
        ImageWorker::in_process(),
    )
    .with_store(StoreConfig {
        index_dir: Some(index_dir),
        community: false,
    })
}
