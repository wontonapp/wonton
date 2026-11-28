//! 商店来源：加上的来源出现在商店里，重复添加被拒时如实说原因，移除发对地址。

use axum::http::Method;
use wonton_client::{AddedSource, CoreError, SourceKind, StoreSection};

use super::rig::{Canned, Rig, core_error, item, listing};

#[tokio::test]
async fn an_added_source_shows_up_in_the_store_and_a_refused_one_says_why() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = "/_wonton/api/store/sources";

    rig.core.answer(
        Method::POST,
        path,
        core_error(409, "conflict", "已经添加过"),
    );
    let refused = rig
        .app
        .add_store_source("o/r".to_owned())
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::NotReady { .. }), "{refused}");

    let added = AddedSource {
        address: "github:o/r".to_owned(),
        kind: SourceKind::Repo,
        provider: "github.com/o/r".to_owned(),
        name: None,
        section: StoreSection {
            plugins: vec![item("app.added", "Added", &[])],
            fetched_at_ms: None,
            problem: None,
            unreadable: Vec::new(),
        },
    };
    rig.core.answer(
        Method::POST,
        path,
        Canned(201, serde_json::to_string(&added).unwrap()),
    );
    let mut store = listing();
    store.added = vec![added];
    rig.core.answer(
        Method::GET,
        "/_wonton/api/store",
        Canned(200, serde_json::to_string(&store).unwrap()),
    );
    rig.app.add_store_source("o/r".to_owned()).await.unwrap();
    rig.wait_for(|view| view.store.as_ref().is_some_and(|s| !s.added.is_empty()))
        .await;
    assert_eq!(rig.app.search_store("added".to_owned())[0].id, "app.added");

    rig.app
        .remove_store_source("github:o/r".to_owned())
        .await
        .unwrap();
    assert_eq!(
        rig.core
            .seen(
                &Method::DELETE,
                "/_wonton/api/store/sources?address=github%3Ao%2Fr"
            )
            .len(),
        1
    );
}
