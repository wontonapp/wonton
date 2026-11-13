use std::collections::HashMap;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use bollard::models::ContainerCreateBody;
use serde_json::json;

use super::*;
use crate::members::MemberRecord;
use crate::plugin::fake_docker::FakeDocker;

async fn compose(engine: &Engine, name: &str, project: &str, service: &str) {
    engine
        .create_container(
            name,
            ContainerCreateBody {
                image: Some("mailpit-test".into()),
                labels: Some(HashMap::from([
                    ("com.docker.compose.project".into(), project.into()),
                    ("com.docker.compose.service".into(), service.into()),
                ])),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

fn open(engine: Engine, dir: &Path, members: Arc<Members>) -> Arc<Component> {
    let meta = fs::metadata(dir).unwrap();
    Component::open(
        engine,
        "core".into(),
        dir,
        "alice",
        members,
        meta.uid(),
        meta.gid(),
    )
    .unwrap()
}

fn invite(members: &Members, name: &str) {
    members
        .invite(MemberRecord {
            username: name.into(),
            user_id: format!("@{name}:home"),
            display_name: name.into(),
            email: None,
            status: MemberStatus::Invited,
            role_id: "friend".to_owned(),
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
}

#[tokio::test]
async fn recipients_are_exactly_owner_and_joined_people() {
    let dir = tempfile::tempdir().unwrap();
    let records = json!({"members": [
        {"username": "bob", "user_id": "@bob:home", "display_name": "Bob", "status": "joined"},
        {"username": "carol", "user_id": "@carol:home", "display_name": "Carol", "status": "invited"},
        {"username": "dave", "user_id": "@dave:home", "display_name": "Dave", "status": "removed"}
    ]});
    fs::write(
        dir.path().join("members.json"),
        serde_json::to_vec(&records).unwrap(),
    )
    .unwrap();
    let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
    let (_, engine) = FakeDocker::start().await;
    let component = open(engine, dir.path(), members);
    let content = fs::read_to_string(component.config_dir.join("runtime.env")).unwrap();
    let rule = content
        .strip_prefix("export MP_SMTP_ALLOWED_RECIPIENTS='")
        .unwrap()
        .strip_suffix("'\n")
        .unwrap();
    let regex = regex::Regex::new(rule).unwrap();
    for address in ["alice@wonton.internal", "bob@wonton.internal"] {
        assert!(regex.is_match(address), "{address}");
    }
    for address in [
        "carol@wonton.internal",
        "dave@wonton.internal",
        "nobody@wonton.internal",
        "bob@example.com",
        "app.test@plugins.wonton.internal",
        "alice@wontonXinternal",
        "xalice@wonton.internal",
        "alice@wonton.internal.evil",
    ] {
        assert!(!regex.is_match(address), "{address}");
    }
    assert!(dir.path().join("mailpit/data").is_dir());
    assert_eq!(
        fs::metadata(dir.path().join("mailpit/api-password"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::read_to_string(component.config_dir.join("auth.htpasswd")).unwrap(),
        format!("core:{}\n", component.password())
    );
}

#[tokio::test]
async fn failed_restart_remains_pending_and_preserves_database_and_password() {
    let dir = tempfile::tempdir().unwrap();
    let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
    let (docker, engine) = FakeDocker::start().await;
    compose(&engine, "core", "home", "core").await;
    compose(&engine, "mail", "home", "mailpit").await;
    let component = open(engine.clone(), dir.path(), members.clone());
    let database = dir.path().join("mailpit/data/mailpit.db");
    fs::write(&database, b"SQLite survives").unwrap();
    // Invited 没有事件也不能先放行；此处模拟磁盘里有旧名单，触发一次 reload。
    invite(&members, "bob");
    fs::write(component.config_dir.join("runtime.env"), "stale").unwrap();
    docker.fail_starts(Some("injected restart failure"));
    assert!(component.refresh_recipients().await.is_err());
    assert!(*component.restart_pending.lock().await);
    docker.fail_starts(None);
    component.refresh_recipients().await.unwrap();
    assert!(!*component.restart_pending.lock().await);
    assert_eq!(
        docker
            .writes()
            .iter()
            .filter(|call| *call == "POST /containers/mail/start")
            .count(),
        2
    );
    assert_eq!(fs::read(&database).unwrap(), b"SQLite survives");
    assert!(
        !docker
            .writes()
            .iter()
            .any(|call| call == "DELETE /containers/mail")
    );
    members.consume_code("bob").unwrap();
    component.refresh_recipients().await.unwrap();
    assert!(
        fs::read_to_string(component.config_dir.join("runtime.env"))
            .unwrap()
            .contains("alice|bob")
    );
    members.remove("bob", false, false).unwrap();
    component.refresh_recipients().await.unwrap();
    assert!(
        !fs::read_to_string(component.config_dir.join("runtime.env"))
            .unwrap()
            .contains("bob")
    );
    assert_eq!(fs::read(&database).unwrap(), b"SQLite survives");
    let reopened = open(engine, dir.path(), members);
    assert_eq!(reopened.password(), component.password());
    assert!(*reopened.restart_pending.lock().await);
    reopened.refresh_recipients().await.unwrap();
}

#[tokio::test]
async fn lookup_never_uses_another_compose_project_and_rejects_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
    let (_, engine) = FakeDocker::start().await;
    compose(&engine, "core", "home", "core").await;
    compose(&engine, "foreign-mail", "other", "mailpit").await;
    let component = open(engine.clone(), dir.path(), members);
    assert!(component.container().await.is_err());
    compose(&engine, "our-mail", "home", "mailpit").await;
    assert_eq!(component.container().await.unwrap(), "our-mail");
    compose(&engine, "duplicate-mail", "home", "mailpit").await;
    assert!(component.container().await.is_err());
}

#[tokio::test]
async fn a_self_signed_starttls_certificate_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
    let (_, engine) = FakeDocker::start().await;
    let component = open(engine.clone(), dir.path(), members.clone());
    let cert = component.config_dir.join("smtp-tls.crt");
    let key = component.config_dir.join("smtp-tls.key");
    for path in [&cert, &key] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // 证书的 SAN 是插件网络里的别名，客户端按它校验证书。
    let mut reader = std::io::BufReader::new(fs::File::open(&cert).unwrap());
    let certificate = rustls_pemfile::certs(&mut reader).next().unwrap().unwrap();
    assert!(
        certificate
            .windows("smtp.wonton.internal".len())
            .any(|window| window == b"smtp.wonton.internal")
    );

    // 重建（升级、重装）不换证书，插件应用里存的信任不用跟着改。
    let before = fs::read_to_string(&cert).unwrap();
    drop(component);
    open(engine, dir.path(), members);
    assert_eq!(fs::read_to_string(&cert).unwrap(), before);
}
