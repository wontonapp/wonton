use serde_json::json;

use super::*;
use crate::{Action, BotAction, Rgb};

const USER_ID: &str = "user:6f1c2d3e-4b5a-4c7d-8e9f-0a1b2c3d4e5f";

fn cloud() -> BotCloud {
    BotCloud {
        body: "#C8DBE9".into(),
        face: "builtin:face/smile".into(),
        headdress: Some("builtin:headdress/spiral".into()),
    }
}

#[test]
fn the_profile_field_has_a_stable_wire_shape() {
    assert_eq!(
        serde_json::to_value(cloud()).unwrap(),
        json!({"body": "#C8DBE9", "face": "builtin:face/smile", "headdress": "builtin:headdress/spiral"})
    );
    // 不戴头饰时不写这个键；Agent 写的 null 也读成不戴。
    let bare: BotCloud =
        serde_json::from_value(json!({"body": "#c8dbe9", "face": USER_ID, "headdress": null}))
            .unwrap();
    assert_eq!(bare.headdress, None);
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        json!({"body": "#c8dbe9", "face": USER_ID})
    );
}

#[test]
fn any_color_value_is_accepted_and_written_back_in_one_form() {
    assert_eq!(
        parse_cloud_color("#c8dbe9"),
        Some(Rgb {
            red: 0xC8,
            green: 0xDB,
            blue: 0xE9
        })
    );
    for bad in [
        "C8DBE9",
        "#C8DBE",
        "#C8DBE9FF",
        "#GGGGGG",
        "",
        "#",
        "#+1+2+3",
    ] {
        assert_eq!(parse_cloud_color(bad), None, "{bad}");
    }
    assert_eq!(
        cloud_color(Rgb {
            red: 0x0a,
            green: 0xff,
            blue: 0
        }),
        "#0AFF00"
    );
}

#[test]
fn asset_ids_are_namespaced() {
    assert_eq!(
        parse_cloud_asset_id("builtin:face/smile"),
        Some(CloudAssetRef::Builtin {
            layer: CloudLayer::Face,
            key: "smile"
        })
    );
    assert!(matches!(
        parse_cloud_asset_id(USER_ID),
        Some(CloudAssetRef::User { .. })
    ));
    for bad in [
        "smile",
        "builtin:face",
        "builtin:face/",
        "builtin:hat/chef",
        "builtin:face/Smile",
        "builtin:face/a/b",
        "user:not-a-uuid",
        "user:6F1C2D3E-4B5A-4C7D-8E9F-0A1B2C3D4E5F",
        "user:6f1c2d3e4b5a4c7d8e9f0a1b2c3d4e5f",
        "plugin:6f1c2d3e-4b5a-4c7d-8e9f-0a1b2c3d4e5f",
    ] {
        assert_eq!(parse_cloud_asset_id(bad), None, "{bad}");
    }
}

#[test]
fn a_cloud_is_checked_part_by_part() {
    assert_eq!(cloud().check(), Ok(()));
    let user_parts = BotCloud {
        body: "#000000".into(),
        face: USER_ID.into(),
        headdress: None,
    };
    assert_eq!(user_parts.check(), Ok(()));
    let mut bad_color = cloud();
    bad_color.body = "sky".into();
    assert_eq!(bad_color.check(), Err(CloudProblem::Color));
    let mut hat_as_face = cloud();
    hat_as_face.face = "builtin:headdress/spiral".into();
    assert_eq!(hat_as_face.check(), Err(CloudProblem::WrongKind));
    let mut garbage_hat = cloud();
    garbage_hat.headdress = Some("chef hat".into());
    assert_eq!(garbage_hat.check(), Err(CloudProblem::AssetId));
}

#[test]
fn asset_paths_keep_the_id_in_one_segment() {
    assert_eq!(
        cloud_asset_path("builtin:face/smile"),
        "/_wonton/api/cloud-assets/builtin%3Aface%2Fsmile"
    );
}

#[test]
fn asset_names_follow_the_import_rules() {
    assert_eq!(check_cloud_asset_name("厨师帽"), Ok(()));
    assert_eq!(
        check_cloud_asset_name(""),
        Err(CloudAssetNameProblem::Empty)
    );
    assert_eq!(
        check_cloud_asset_name(&"帽".repeat(MAX_CLOUD_ASSET_NAME_CHARS)),
        Ok(())
    );
    assert_eq!(
        check_cloud_asset_name(&"帽".repeat(MAX_CLOUD_ASSET_NAME_CHARS + 1)),
        Err(CloudAssetNameProblem::TooLong)
    );
    assert_eq!(
        check_cloud_asset_name("厨师\n帽"),
        Err(CloudAssetNameProblem::ControlCharacters)
    );
}

#[test]
fn unknown_asset_kinds_authors_and_problems_fall_to_other() {
    let list: CloudAssets = serde_json::from_value(json!({"assets": [
        {"id": "builtin:body/sky", "kind": "body_preset", "name": "浅天蓝", "author": {"type": "builtin"}, "color": "#C8DBE9"},
        {"id": USER_ID, "kind": "face", "name": "眨眼", "author": {"type": "plugin", "plugin_id": "app.example.ledger"}},
        {"id": "builtin:shoes/boot", "kind": "shoes", "name": "靴子", "author": {"type": "studio"}}
    ]}))
    .unwrap();
    assert_eq!(list.assets[0].kind, CloudAssetKind::BodyPreset);
    assert_eq!(list.assets[0].color.as_deref(), Some("#C8DBE9"));
    assert_eq!(
        list.assets[1].author,
        CloudAssetAuthor::Plugin {
            plugin_id: "app.example.ledger".into()
        }
    );
    assert_eq!(list.assets[2].kind, CloudAssetKind::Other);
    assert_eq!(list.assets[2].author, CloudAssetAuthor::Other);
    let problem: CloudProblem = serde_json::from_value(json!("too_bright")).unwrap();
    assert_eq!(problem, CloudProblem::Other);
}

#[test]
fn cloud_actions_round_trip_through_json() {
    for action in [
        BotAction::SetBotCloud {
            plugin_id: "app.example.ledger".into(),
            cloud: cloud(),
        },
        BotAction::SetBotCloudPart {
            plugin_id: "app.example.ledger".into(),
            part: CloudPart::Body {
                color: "#FBC09B".into(),
            },
        },
        BotAction::SetBotCloudPart {
            plugin_id: "app.example.ledger".into(),
            part: CloudPart::Headdress { id: None },
        },
    ] {
        let action = Action::Bot { action };
        let value = serde_json::to_value(&action).unwrap();
        assert_eq!(serde_json::from_value::<Action>(value).unwrap(), action);
    }
    assert_eq!(
        serde_json::to_value(CloudPart::Face {
            id: "builtin:face/tongue".into()
        })
        .unwrap(),
        json!({"part": "face", "id": "builtin:face/tongue"})
    );
}
