use serde_json::json;
use sha2::{Digest, Sha256};
use wonton_protocol::MAIL_FIELD;

use super::delivery::Delivery;
use super::test_support::{Mailpit, home, received};

#[tokio::test]
async fn original_html_and_small_files_follow_mail_but_oversized_files_only_remain_in_metadata() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    mailpit.add(
        "files",
        json!([{"Address":"owner@wonton.internal"}]),
        received("172.30.1.5"),
    );
    let html = format!("<p>{}</p>", "云".repeat(4001));
    {
        let mut state = mailpit.state.lock().unwrap();
        let message = state.messages.get_mut("files").unwrap();
        message["Text"] = json!("云".repeat(4001));
        message["HTML"] = json!(html);
        message["Attachments"] = json!([
            {"PartID":"1.1","FileName":"reset.txt","ContentType":"text/plain","Size":5},
            {"PartID":"1.2","FileName":"large.zip","ContentType":"application/zip","Size":20_971_521},
        ]);
        state
            .parts
            .insert(("files".into(), "1.1".into()), b"reset".to_vec());
    }
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    delivery.deliver("files").await.unwrap();
    let matrix = home.matrix.state();
    assert_eq!(matrix.sent.len(), 3);
    let mail = &matrix.sent[0].1;
    assert_eq!(mail["msgtype"], "m.text");
    assert_eq!(mail[MAIL_FIELD]["body"], "云".repeat(4000));
    assert_eq!(
        mail[MAIL_FIELD]["attachments"],
        json!([
            {"name":"reset.txt","size":5,"uploaded":true},
            {"name":"large.zip","size":20_971_521,"uploaded":false},
            {"name":"message.html","size":html.len(),"uploaded":true},
        ])
    );
    assert_eq!(
        matrix.media,
        [
            ("text/plain".into(), b"reset".to_vec()),
            ("text/html".into(), html.into_bytes()),
        ]
    );
    for ((room, content), name) in matrix.sent[1..].iter().zip(["reset.txt", "message.html"]) {
        assert_eq!(room, &matrix.sent[0].0);
        assert_eq!(content["msgtype"], "m.file");
        assert_eq!(content["filename"], name);
        let uri = content["url"].as_str().unwrap();
        assert!(matrix.media_ids.contains_key(uri));
    }
    assert_eq!(mailpit.state.lock().unwrap().deleted, ["files"]);
}

#[tokio::test]
async fn a_missing_attachment_keeps_the_mail_and_sends_nothing_until_recovery() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    mailpit.add(
        "missing-file",
        json!([{"Address":"owner@wonton.internal"}]),
        received("172.30.1.5"),
    );
    mailpit
        .state
        .lock()
        .unwrap()
        .messages
        .get_mut("missing-file")
        .unwrap()["Attachments"] = json!([
        {"PartID":"1.1","FileName":"reset.txt","ContentType":"text/plain","Size":5},
    ]);
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    assert!(delivery.deliver("missing-file").await.is_err());
    assert!(mailpit.state.lock().unwrap().deleted.is_empty());
    assert!(home.matrix.state().sent.is_empty());
    mailpit
        .state
        .lock()
        .unwrap()
        .parts
        .insert(("missing-file".into(), "1.1".into()), b"reset".to_vec());
    delivery.deliver("missing-file").await.unwrap();
    assert_eq!(mailpit.state.lock().unwrap().deleted, ["missing-file"]);
    let state = home.matrix.state();
    assert_eq!(state.sent.len(), 2);
    assert_eq!(state.sent[0].1["msgtype"], "m.text");
    assert_eq!(state.sent[1].1["filename"], "reset.txt");
}

#[tokio::test]
async fn an_html_body_that_cannot_be_converted_still_delivers_with_the_original_attached() {
    // 20000 层嵌套的 blockquote 会让 html2text 报 TooNarrow，这条错误重试多少次都一样：
    // 要当场换成固定提示、把原文附上继续投递，不能卡在每 5 秒重扫里等七天后被 Mailpit 删掉。
    let home = home().await;
    let mailpit = Mailpit::start().await;
    mailpit.add(
        "narrow",
        json!([{"Address":"owner@wonton.internal"}]),
        received("172.30.1.5"),
    );
    let html = format!(
        "{}x{}",
        "<blockquote>".repeat(20_000),
        "</blockquote>".repeat(20_000)
    );
    {
        let mut state = mailpit.state.lock().unwrap();
        let message = state.messages.get_mut("narrow").unwrap();
        message["Text"] = json!("");
        message["HTML"] = json!(html);
    }
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    delivery.deliver("narrow").await.unwrap();
    let matrix = home.matrix.state();
    let mail = &matrix.sent[0].1;
    assert_eq!(
        mail[MAIL_FIELD]["body"],
        super::content::CONVERT_FAILED_BODY
    );
    assert_eq!(
        mail[MAIL_FIELD]["attachments"],
        json!([{"name":"message.html","size":html.len(),"uploaded":true}])
    );
    assert_eq!(matrix.media, [("text/html".into(), html.into_bytes())]);
    assert_eq!(mailpit.state.lock().unwrap().deleted, ["narrow"]);
    // 投完就删，进度也清掉：不会再被重扫捡起来。
    let record = format!("{:x}.json", Sha256::digest(b"narrow"));
    assert!(!delivery.progress.dir().join(record).exists());
}
