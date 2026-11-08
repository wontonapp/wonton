use matrix_sdk::{
    deserialized_responses::{TimelineEvent, UnableToDecryptInfo, UnableToDecryptReason},
    ruma::{room_id, serde::Raw, user_id},
};
use matrix_sdk_crypto::{EncryptionSettings, OlmMachine, encrypt_room_key_export};
use serde_json::{Value, json};

pub(super) struct EncryptedMessages {
    pub earlier: Value,
    pub latest: Value,
    pub keys: tempfile::NamedTempFile,
}

impl EncryptedMessages {
    pub async fn create() -> Self {
        let room = room_id!("!bootstrap:example.org");
        let alice = OlmMachine::new(user_id!("@alice:example.org"), "ALICE".into()).await;
        alice
            .share_room_key(room, std::iter::empty(), EncryptionSettings::default())
            .await
            .unwrap();
        let mut events = Vec::new();
        for (id, content) in [
            (
                "$earlier",
                json!({"msgtype":"m.text", "body":"private text"}),
            ),
            (
                "$latest",
                json!({"msgtype":"m.image", "body":"private.jpg", "url":"mxc://example.org/private"}),
            ),
        ] {
            let encrypted = alice
                .encrypt_room_event_raw(
                    room,
                    "m.room.message",
                    &Raw::new(&content).unwrap().cast_unchecked(),
                )
                .await
                .unwrap();
            events.push(json!({
                "type":"m.room.encrypted", "event_id":id, "room_id":room,
                "sender":"@alice:example.org", "origin_server_ts":1,
                "content":encrypted.content
            }));
        }
        let exported = alice.store().export_room_keys(|_| true).await.unwrap();
        let keys = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            keys.path(),
            encrypt_room_key_export(&exported, "test", 10_000).unwrap(),
        )
        .unwrap();
        Self {
            earlier: events.remove(0),
            latest: events.remove(0),
            keys,
        }
    }

    pub fn undecryptable(event: &Value) -> TimelineEvent {
        TimelineEvent::from_utd(
            Raw::new(event).unwrap().cast_unchecked(),
            UnableToDecryptInfo {
                session_id: event["content"]["session_id"].as_str().map(str::to_owned),
                reason: UnableToDecryptReason::MissingMegolmSession {
                    withheld_code: None,
                },
            },
        )
    }
}
