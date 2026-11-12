use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::owner::{Owner, OwnerStore};
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::names::Instance;
use crate::plugin::store::Store;
use crate::plugin::storefront::StoreConfig;

pub(super) fn manager(engine: Engine, dir: &tempfile::TempDir) -> PluginManager {
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
        install_timeout: Duration::from_secs(30),
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
        instance: Instance::parse(Some("emcm")).unwrap(),
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
