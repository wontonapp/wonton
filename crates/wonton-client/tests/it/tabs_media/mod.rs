use base64::Engine;
use serde_json::{Value, json};
use wonton_client::{ClientCore, Network, PageMediaAction, PageMediaPlayback};

mod script;

fn core() -> (tempfile::TempDir, std::sync::Arc<ClientCore>) {
    let directory = tempfile::tempdir().unwrap();
    let core =
        ClientCore::with_network(directory.path().to_str().unwrap(), Network::DirectOnly).unwrap();
    (directory, core)
}

fn message() -> Value {
    json!({
        "version": 1, "playback": "playing", "title": "Track", "artist": "Artist",
        "album": "Album", "artwork": "AQID", "duration_ms": 60000,
        "position_ms": 12000, "rate": 1.5,
        "actions": ["play", "pause", "nexttrack", "seekto"]
    })
}

#[test]
fn maps_web_media_state_to_the_system_contract() {
    let (_directory, core) = core();
    let state = core.page_media_message(message().to_string()).unwrap();
    assert_eq!(state.playback, PageMediaPlayback::Playing);
    assert_eq!(
        (
            state.title.as_str(),
            state.artist.as_str(),
            state.album.as_str()
        ),
        ("Track", "Artist", "Album")
    );
    assert_eq!(state.artwork, Some(vec![1, 2, 3]));
    assert_eq!(
        (state.duration_ms, state.position_ms, state.rate),
        (Some(60000), Some(12000), 1.5)
    );
    assert_eq!(
        state.actions,
        [
            PageMediaAction::Play,
            PageMediaAction::Pause,
            PageMediaAction::NextTrack,
            PageMediaAction::SeekTo
        ]
    );
}

#[test]
fn drops_unknown_and_duplicate_actions_without_losing_known_actions() {
    let (_directory, core) = core();
    let mut message = message();
    message["actions"] = json!(["nexttrack", "future", "pause", "nexttrack"]);
    message["playback"] = json!("future");
    let state = core.page_media_message(message.to_string()).unwrap();
    assert_eq!(
        state.actions,
        [PageMediaAction::NextTrack, PageMediaAction::Pause]
    );
    assert_eq!(state.playback, PageMediaPlayback::Other);
}

#[test]
fn rejects_invalid_json_versions_positions_and_artwork() {
    let (_directory, core) = core();
    assert!(core.page_media_message("{".into()).is_none());
    for (field, value) in [
        ("version", json!(2)),
        ("rate", json!(0)),
        ("rate", json!(-1)),
        ("artwork", json!("not base64!")),
        (
            "artwork",
            json!(base64::engine::general_purpose::STANDARD.encode(vec![0; 512 * 1024 + 1])),
        ),
    ] {
        let mut message = message();
        message[field] = value;
        assert!(
            core.page_media_message(message.to_string()).is_none(),
            "{field}"
        );
    }
}

#[test]
fn clamps_position_at_duration() {
    let (_directory, core) = core();
    let mut message = message();
    message["position_ms"] = json!(60001);
    let state = core.page_media_message(message.to_string()).unwrap();
    assert_eq!(state.position_ms, Some(60000));
}

#[test]
fn accepts_paused_live_media_without_cover_or_duration() {
    let (_directory, core) = core();
    let mut message = message();
    message["playback"] = json!("paused");
    message["duration_ms"] = Value::Null;
    message["artwork"] = Value::Null;
    let state = core.page_media_message(message.to_string()).unwrap();
    assert_eq!(state.playback, PageMediaPlayback::Paused);
    assert_eq!(state.duration_ms, None);
    assert_eq!(state.artwork, None);
    assert_eq!(state.position_ms, Some(12000));
}
