use super::*;

fn home_mount(bed: &Bed, bot: &BotRef) -> Value {
    let container =
        crate::plugin::names::Instance::default().bot_container("app-echo", "bot", &bot.bot);
    let created = bed.docker.created(&container).unwrap();
    created["HostConfig"]["Mounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mount| mount["Target"] == "/wonton/home")
        .unwrap()
        .clone()
}

#[tokio::test]
async fn ai_role_changes_reconcile_the_private_bot_mount_without_changing_its_identity() {
    let bed = Bed::start().await;
    let (id, _) = bed.pending("alice", true);
    let reply = bed.approve(&id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let bot = bed.bot("alice");
    let initial = home_mount(&bed, &bot);
    assert_eq!(
        initial["Source"],
        bed.home("alice").to_string_lossy().as_ref()
    );
    for (ai, read_only) in [("read_only", true), ("self_confirms", false)] {
        let reply = bed
            .call(
                Method::PUT,
                &wonton_protocol::role_path("family"),
                json!({
                    "name": "Private members", "permissions": {"private_bot": true, "ai": ai}
                }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        assert_eq!(bed.bot("alice"), bot);
        let mounted = home_mount(&bed, &bot);
        assert_eq!(mounted["Source"], initial["Source"]);
        assert_eq!(mounted["ReadOnly"], read_only);
    }
}

#[tokio::test]
async fn a_removed_member_can_explicitly_delete_data_that_was_kept() {
    let bed = Bed::start().await;
    let (id, _) = bed.pending("alice", true);
    let reply = bed.approve(&id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let bot = bed.bot("alice");
    let data = bed.dir.path().join("apps/app-echo/bots").join(&bot.bot);
    fs::write(data.join("memory"), b"keep then delete").unwrap();
    fs::write(bed.home("alice").join("file"), b"private").unwrap();
    for (delete_home, keep_bot_data) in [(false, true), (true, false)] {
        let reply = bed
            .call(
                Method::POST,
                &wonton_protocol::member_removal_path("alice"),
                json!({
                    "delete_home": delete_home, "keep_bot_data": keep_bot_data,
                }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        assert_eq!(bed.home("alice").exists(), !delete_home);
        assert_eq!(data.join("memory").exists(), keep_bot_data);
    }
}
