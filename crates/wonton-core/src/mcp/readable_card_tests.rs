use std::time::Duration;

use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{Value, json};
use wonton_protocol::{
    Answer, ConfirmationCard, ConfirmationCategory, ConfirmationTool, ConfirmationToolImpact,
};

use super::test_setup::{Setup, setup};
use crate::plugin::docker::Engine;

async fn home(tool: Tool) -> Setup {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    *s.notes.tools.lock().unwrap() = Some(vec![tool]);
    s
}

fn details(card: &ConfirmationCard) -> &ConfirmationTool {
    let Some(ConfirmationCategory::ToolCall { tool }) = &card.category else {
        panic!("missing tool-call category: {:?}", card.category);
    };
    tool
}

#[tokio::test]
async fn an_ordinary_tool_keeps_its_name_and_complete_arguments_out_of_the_main_text() {
    let tool = Tool::new("delete", "", rmcp::model::JsonObject::new());
    let s = home(tool.clone()).await;
    let args = json!({"id": "item-42", "note": "界".repeat(700)});
    let call = s.spawn("app-example-notes__delete", args.clone());
    let card = s.cards.wait_for(1).await;
    assert_eq!(card.title, tool.name);
    assert!(card.lines.is_empty());
    let shown = details(&card);
    assert_eq!(shown.name, tool.name);
    assert_eq!(
        serde_json::from_str::<Value>(&shown.arguments).unwrap(),
        args
    );
    assert_eq!(shown.impact, ConfirmationToolImpact::ChangesData);
    assert!(!shown.runs_as_root);
    assert!(s.notes.deleted.lock().unwrap().is_empty());
    s.answer(&card.id, Answer::Reject).unwrap();
    assert_eq!(call.await.unwrap().is_error, Some(true));
    assert!(s.notes.deleted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn provided_titles_and_descriptions_reach_the_card_in_precedence_order() {
    let tool = Tool::new(
        "delete",
        "Provider description",
        rmcp::model::JsonObject::new(),
    )
    .with_title("Provider title")
    .with_annotations(ToolAnnotations::with_title("Annotation title"));
    let cases = [
        (
            Some("Provider title"),
            Some("Annotation title"),
            Some("Provider description"),
        ),
        (
            Some("  "),
            Some("Annotation title"),
            Some("Provider description"),
        ),
        (None, Some("  "), Some("Provider description")),
        (None, None, None),
    ];
    let s = home(tool.clone()).await;
    for (index, (title, annotation, description)) in cases.into_iter().enumerate() {
        let mut definition = tool.clone();
        definition.title = title.map(str::to_owned);
        definition.annotations.as_mut().unwrap().title = annotation.map(str::to_owned);
        definition.description = description.map(Into::into);
        let expected = title
            .filter(|s| !s.trim().is_empty())
            .or(annotation.filter(|s| !s.trim().is_empty()))
            .or(description)
            .unwrap_or(&tool.name);
        *s.notes.tools.lock().unwrap() = Some(vec![definition]);
        let call = s.spawn(
            "app-example-notes__delete",
            json!({"name": "original-target"}),
        );
        let card = s.cards.wait_for(index + 1).await;
        assert_eq!(card.title, expected);
        assert_eq!(
            card.lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            description
                .filter(|text| *text != expected)
                .into_iter()
                .collect::<Vec<_>>()
        );
        s.answer(&card.id, Answer::Approve).unwrap();
        assert_ne!(call.await.unwrap().is_error, Some(true));
    }
    assert_eq!(
        *s.notes.deleted.lock().unwrap(),
        vec!["original-target"; cases.len()]
    );
}

#[tokio::test]
async fn a_destructive_tool_never_invents_a_target_from_callers_arguments() {
    let tool = Tool::new("purge", "Remove item", rmcp::model::JsonObject::new())
        .with_annotations(ToolAnnotations::new().read_only(false).destructive(true));
    let s = home(tool).await;
    let args = json!({"id": "opaque-42", "name": "caller-supplied label"});
    let call = s.spawn("app-example-notes__purge", args.clone());
    let card = s.cards.wait_for(1).await;
    assert_eq!(
        details(&card).impact,
        ConfirmationToolImpact::Destructive { target: None }
    );
    assert_eq!(
        serde_json::from_str::<Value>(&details(&card).arguments).unwrap(),
        args
    );
    assert!(
        !card
            .choices
            .iter()
            .any(|choice| choice.answer == Answer::AllowWindow)
    );
    assert!(s.notes.deleted.lock().unwrap().is_empty());
    s.answer(&card.id, Answer::Reject).unwrap();
    assert_eq!(call.await.unwrap().is_error, Some(true));
    assert!(s.notes.deleted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstalling_uses_the_installed_objects_localized_name() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    s.manager.note_person_languages("owner", Some("en"));
    let id = "app.example.notes";
    let expected = s
        .manager
        .list()
        .into_iter()
        .find(|plugin| plugin.id.as_str() == id)
        .unwrap()
        .title["en"]
        .clone();
    let args = json!({"id": id, "delete_data": true});
    let call = s.spawn("wonton__uninstall_plugin", args.clone());
    let card = s.cards.wait_for(1).await;
    assert_eq!(
        details(&card).impact,
        ConfirmationToolImpact::Destructive {
            target: Some(expected.clone())
        }
    );
    assert_eq!(details(&card).plugin_title, expected);
    assert_eq!(
        serde_json::from_str::<Value>(&details(&card).arguments).unwrap(),
        args
    );
    s.answer(&card.id, Answer::Reject).unwrap();
    assert_eq!(call.await.unwrap().is_error, Some(true));
    assert!(
        s.manager
            .list()
            .iter()
            .any(|plugin| plugin.id.as_str() == id)
    );
}
