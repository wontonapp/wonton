//! 收件箱条目的字段校验（ADR-0119）：核心建条目和作答时都先过这里，超限报 `invalid_input`，不截断。
//!
//! 只校验形状和上限，不判断权限、期限和上限条数（那些在核心的收件箱实现里）。错误带字段名，壳能指到哪一项。

use std::collections::HashSet;

use crate::inbox::{
    ChoicePayload, ConfirmAction, ConfirmPayload, InboxAction, InboxActionStyle, InboxAnswerAction,
    InboxAnswerInput, InboxItem, InboxOption, InboxPayload, InboxQuestion, InboxRisk, InboxStatus,
    InboxType, MAX_INBOX_ACTION_LABEL_CHARS, MAX_INBOX_ACTIONS, MAX_INBOX_BODY_CHARS,
    MAX_INBOX_CAPTION_CHARS, MAX_INBOX_COMMENT_CHARS, MAX_INBOX_DEDUPE_KEY_CHARS,
    MAX_INBOX_DETAIL_CHARS, MAX_INBOX_DISPLAY_NAME_CHARS, MAX_INBOX_HEADER_CHARS,
    MAX_INBOX_NETWORK_DOMAINS, MAX_INBOX_NOTE_CHARS, MAX_INBOX_OPTION_DESCRIPTION_CHARS,
    MAX_INBOX_OPTION_LABEL_CHARS, MAX_INBOX_OPTIONS, MAX_INBOX_OTHER_CHARS,
    MAX_INBOX_QUESTION_CHARS, MAX_INBOX_QUESTIONS, MAX_INBOX_SENDER_ID_CHARS,
    MAX_INBOX_SUMMARY_CHARS, MAX_INBOX_TITLE_CHARS, MIN_INBOX_OPTIONS,
};

/// 一个字段不合法。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InboxInvalid {
    #[error("缺少 {field}")]
    Missing { field: String },
    #[error("{field} 太长（{actual} 字 > {max}）")]
    TooLong {
        field: String,
        actual: usize,
        max: usize,
    },
    #[error("{field} 太多（{actual} > {max}）")]
    TooMany {
        field: String,
        actual: usize,
        max: usize,
    },
    #[error("{field} 太少（{actual} < {min}）")]
    TooFew {
        field: String,
        actual: usize,
        min: usize,
    },
    #[error("{field}={value:?} 不合法：{reason}")]
    Invalid {
        field: String,
        value: String,
        reason: String,
    },
    #[error("{field} 里的 {id:?} 重复")]
    Duplicate { field: String, id: String },
    #[error("{field} 和条目类型不一致")]
    Mismatch { field: String },
}

impl InboxInvalid {
    #[must_use]
    pub fn field(&self) -> &str {
        match self {
            Self::Missing { field }
            | Self::TooLong { field, .. }
            | Self::TooMany { field, .. }
            | Self::TooFew { field, .. }
            | Self::Invalid { field, .. }
            | Self::Duplicate { field, .. }
            | Self::Mismatch { field } => field,
        }
    }
}

fn long(field: &str, value: &str, max: usize) -> Result<(), InboxInvalid> {
    let actual = value.chars().count();
    if actual > max {
        return Err(InboxInvalid::TooLong {
            field: field.to_owned(),
            actual,
            max,
        });
    }
    Ok(())
}

fn present(field: &str, value: &str) -> Result<(), InboxInvalid> {
    if value.trim().is_empty() {
        return Err(InboxInvalid::Missing {
            field: field.to_owned(),
        });
    }
    Ok(())
}

fn many(field: &str, count: usize, max: usize) -> Result<(), InboxInvalid> {
    if count > max {
        return Err(InboxInvalid::TooMany {
            field: field.to_owned(),
            actual: count,
            max,
        });
    }
    Ok(())
}

/// 校验一整条条目：类型和载荷一致、公共字段、各类载荷的上限和形状。
pub fn validate_item(item: &InboxItem) -> Result<(), InboxInvalid> {
    validate_id("id", &item.id)?;
    let prefix = item.id.chars().next().unwrap_or('?');
    if let Some(kind) = InboxType::from_prefix(prefix)
        && kind != item.kind
    {
        return Err(InboxInvalid::Mismatch {
            field: "id 前缀和 type 不一致".to_owned(),
        });
    }
    present("title", &item.title)?;
    long("title", &item.title, MAX_INBOX_TITLE_CHARS)?;
    long("body", &item.body, MAX_INBOX_BODY_CHARS)?;
    present("sender.id", &item.sender.id)?;
    long("sender.id", &item.sender.id, MAX_INBOX_SENDER_ID_CHARS)?;
    present("sender.display_name", &item.sender.display_name)?;
    long(
        "sender.display_name",
        &item.sender.display_name,
        MAX_INBOX_DISPLAY_NAME_CHARS,
    )?;
    if let Some(key) = &item.dedupe_key {
        long("dedupe_key", key, MAX_INBOX_DEDUPE_KEY_CHARS)?;
    }
    if let Some(progress) = item.progress
        && !(0.0..=1.0).contains(&progress)
    {
        return Err(InboxInvalid::Invalid {
            field: "progress".to_owned(),
            value: progress.to_string(),
            reason: "必须在 0 和 1 之间".to_owned(),
        });
    }
    match (&item.kind, &item.payload) {
        (InboxType::Choice, InboxPayload::Choice(payload)) => validate_choice(payload),
        (InboxType::Page, InboxPayload::Page(payload)) => {
            present("payload.caption", &payload.caption)?;
            long("payload.caption", &payload.caption, MAX_INBOX_CAPTION_CHARS)?;
            let present_sources = usize::from(payload.resource_uri.is_some())
                + usize::from(payload.file_id.is_some());
            if present_sources != 1 {
                return Err(InboxInvalid::Invalid {
                    field: "payload.resource_uri".to_owned(),
                    value: format!("{present_sources} 个来源"),
                    reason: "resource_uri 和 file_id 必须二选一".to_owned(),
                });
            }
            if let Some(uri) = &payload.resource_uri {
                validate_resource_uri(uri)?;
            }
            validate_network_domains(&payload.network_domains)?;
            validate_actions(&payload.actions)?;
            Ok(())
        }
        (InboxType::Recap, InboxPayload::Recap(payload)) => {
            present("payload.summary", &payload.summary)?;
            long("payload.summary", &payload.summary, MAX_INBOX_SUMMARY_CHARS)?;
            if let Some(question) = &payload.question {
                present("payload.question", question)?;
                long("payload.question", question, MAX_INBOX_QUESTION_CHARS)?;
            }
            validate_actions(&payload.actions)?;
            if !(1..=2).contains(&payload.actions.len()) {
                return Err(Invalid::field("payload.actions", "要有 1–2 个动作"));
            }
            Ok(())
        }
        (InboxType::Confirm, InboxPayload::Confirm(payload)) => validate_confirm(payload),
        (InboxType::Notice, InboxPayload::Notice(payload)) => {
            present("payload.title", &payload.title)?;
            long("payload.title", &payload.title, MAX_INBOX_TITLE_CHARS)?;
            present("payload.body", &payload.body)?;
            long("payload.body", &payload.body, MAX_INBOX_BODY_CHARS)?;
            if let Some(link) = &payload.link {
                validate_link(link)?;
            }
            if let Some(action) = &payload.action {
                validate_actions(std::slice::from_ref(action))?;
            }
            Ok(())
        }
        (InboxType::Other, InboxPayload::Other(_)) => Ok(()),
        (kind, payload) => Err(InboxInvalid::Mismatch {
            field: format!("{kind:?} 的条目的载荷是 {:?}", payload.kind()),
        }),
    }
}

/// 校验一个问题卡载荷：1–4 题、题 id 唯一、每题 0 或 2–4 个选项。
pub fn validate_choice(payload: &ChoicePayload) -> Result<(), InboxInvalid> {
    if payload.questions.is_empty() {
        return Err(Invalid::field("payload.questions", "至少 1 题"));
    }
    many(
        "payload.questions",
        payload.questions.len(),
        MAX_INBOX_QUESTIONS,
    )?;
    let mut ids = HashSet::new();
    for question in &payload.questions {
        validate_question(question)?;
        if !ids.insert(question.id.as_str()) {
            return Err(InboxInvalid::Duplicate {
                field: "payload.questions".to_owned(),
                id: question.id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_question(question: &InboxQuestion) -> Result<(), InboxInvalid> {
    present("questions.id", &question.id)?;
    validate_action_id("questions.id", &question.id)?;
    present("questions.header", &question.header)?;
    long("questions.header", &question.header, MAX_INBOX_HEADER_CHARS)?;
    present("questions.question", &question.question)?;
    long(
        "questions.question",
        &question.question,
        MAX_INBOX_QUESTION_CHARS,
    )?;
    if !question.options.is_empty() {
        if question.options.len() < MIN_INBOX_OPTIONS {
            return Err(Invalid::field(
                "questions.options",
                "有选项时至少 2 个；只有一个选项时用 0 个选项加自由输入",
            ));
        }
        many(
            "questions.options",
            question.options.len(),
            MAX_INBOX_OPTIONS,
        )?;
    }
    let mut ids = HashSet::new();
    for option in &question.options {
        validate_option(option)?;
        if !ids.insert(option.id.as_str()) {
            return Err(InboxInvalid::Duplicate {
                field: "questions.options".to_owned(),
                id: option.id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_option(option: &InboxOption) -> Result<(), InboxInvalid> {
    present("options.id", &option.id)?;
    validate_action_id("options.id", &option.id)?;
    present("options.label", &option.label)?;
    long("options.label", &option.label, MAX_INBOX_OPTION_LABEL_CHARS)?;
    if let Some(description) = &option.description {
        long(
            "options.description",
            description,
            MAX_INBOX_OPTION_DESCRIPTION_CHARS,
        )?;
    }
    if let Some(preview) = &option.preview
        && preview.len() > 4096
    {
        return Err(InboxInvalid::TooLong {
            field: "options.preview".to_owned(),
            actual: preview.len(),
            max: 4096,
        });
    }
    Ok(())
}

fn validate_confirm(payload: &ConfirmPayload) -> Result<(), InboxInvalid> {
    present("payload.title", &payload.title)?;
    long("payload.title", &payload.title, MAX_INBOX_TITLE_CHARS)?;
    present("payload.body", &payload.body)?;
    long("payload.body", &payload.body, MAX_INBOX_BODY_CHARS)?;
    if let Some(detail) = &payload.detail {
        long("payload.detail", detail, MAX_INBOX_DETAIL_CHARS)?;
    }
    if matches!(payload.risk, InboxRisk::Other) {
        return Err(Invalid::field("payload.risk", "未知风险档"));
    }
    if payload.choices.len() < 2 {
        return Err(Invalid::field("payload.choices", "至少同意和拒绝两档"));
    }
    let mut saw_approve = false;
    let mut saw_deny = false;
    for choice in &payload.choices {
        present("payload.choices.label", &choice.label)?;
        long(
            "payload.choices.label",
            &choice.label,
            MAX_INBOX_ACTION_LABEL_CHARS,
        )?;
        match choice.action {
            ConfirmAction::Approve => saw_approve = true,
            ConfirmAction::Deny => saw_deny = true,
            _ => {}
        }
    }
    if !saw_approve || !saw_deny {
        return Err(Invalid::field(
            "payload.choices",
            "必须同时有 approve 和 deny",
        ));
    }
    Ok(())
}

fn validate_actions(actions: &[InboxAction]) -> Result<(), InboxInvalid> {
    many("actions", actions.len(), MAX_INBOX_ACTIONS)?;
    let mut ids = HashSet::new();
    for action in actions {
        validate_action_id("actions.id", &action.id)?;
        if !ids.insert(action.id.as_str()) {
            return Err(InboxInvalid::Duplicate {
                field: "actions".to_owned(),
                id: action.id.clone(),
            });
        }
        present("actions.label", &action.label)?;
        long("actions.label", &action.label, MAX_INBOX_ACTION_LABEL_CHARS)?;
        if matches!(action.style, InboxActionStyle::Other) {
            return Err(Invalid::field("actions.style", "未知样式"));
        }
    }
    Ok(())
}

/// 动作或选项 id：`[a-z0-9_-]{1,32}`。
pub fn validate_action_id(field: &str, id: &str) -> Result<(), InboxInvalid> {
    if id.is_empty() || id.len() > 32 {
        return Err(Invalid::field(field, "要 1–32 个字符"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(Invalid::field(field, "只能用小写字母、数字、_ 和 -"));
    }
    Ok(())
}

fn validate_id(_field: &str, id: &str) -> Result<(), InboxInvalid> {
    split_id(id)?;
    Ok(())
}

/// 拆成（前缀字符，随机串）。前缀是任意小写字母，未知前缀不拒。
fn split_id(id: &str) -> Result<(char, &str), InboxInvalid> {
    let mut chars = id.chars();
    let Some(prefix) = chars.next() else {
        return Err(Invalid::field("id", "要“类型前缀-随机串”"));
    };
    if !prefix.is_ascii_lowercase() || chars.next() != Some('-') {
        return Err(Invalid::field("id", "要“类型前缀-随机串”"));
    }
    let rest = &id[prefix.len_utf8() + 1..];
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(Invalid::field("id", "随机串只能用字母、数字、_ 和 -"));
    }
    Ok((prefix, rest))
}

fn validate_resource_uri(uri: &str) -> Result<(), InboxInvalid> {
    if !uri.starts_with(crate::inbox::INBOX_RESOURCE_SCHEME)
        || uri.len() <= crate::inbox::INBOX_RESOURCE_SCHEME.len()
    {
        return Err(Invalid::field("payload.resource_uri", "必须是 ui:// 资源"));
    }
    Ok(())
}

/// 校验并原样返回网络名单：只收 `https://`、`wss://`，可带一个 `*.` 子域通配，不写路径。
pub fn validate_network_domains(domains: &[String]) -> Result<(), InboxInvalid> {
    many(
        "payload.network_domains",
        domains.len(),
        MAX_INBOX_NETWORK_DOMAINS,
    )?;
    let mut seen = HashSet::new();
    for domain in domains {
        let normalized =
            normalize_network_domain(domain).map_err(|reason| InboxInvalid::Invalid {
                field: "payload.network_domains".to_owned(),
                value: domain.clone(),
                reason,
            })?;
        if !seen.insert(normalized) {
            return Err(InboxInvalid::Duplicate {
                field: "payload.network_domains".to_owned(),
                id: domain.clone(),
            });
        }
    }
    Ok(())
}

/// 把一个网络名单写法规范化成 `scheme://host[:port]`，非法时给原因。
pub fn normalize_network_domain(raw: &str) -> Result<String, String> {
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| "要有 https:// 或 wss://".to_owned())?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "wss" {
        return Err("只收 https:// 和 wss://".to_owned());
    }
    if rest.is_empty() || rest.contains(['/', '?', '#', '@']) {
        return Err("只写主机名和可选的端口，不写路径".to_owned());
    }
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    if let Some(port) = port
        && (port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()))
    {
        return Err("端口只能是数字".to_owned());
    }
    let host = host.to_ascii_lowercase();
    let bare = host.strip_prefix("*.").unwrap_or(&host);
    if bare.is_empty() || !bare.contains('.') {
        return Err("主机名要带点".to_owned());
    }
    if !bare.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }) {
        return Err("主机名不合法".to_owned());
    }
    if host.contains("*.") && !host.starts_with("*.") {
        return Err("通配只能整个最左边一段".to_owned());
    }
    Ok(match port {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

/// 链接只收 `https:`、`http:`、`wonton:`。
pub fn validate_link(link: &str) -> Result<(), InboxInvalid> {
    let Some((scheme, rest)) = link.split_once(':') else {
        return Err(Invalid::field(
            "payload.link",
            "要带 https:、http: 或 wonton:",
        ));
    };
    if !matches!(scheme, "https" | "http" | "wonton") || rest.is_empty() {
        return Err(Invalid::field(
            "payload.link",
            "只收 https:、http:、wonton:",
        ));
    }
    Ok(())
}

/// 校验客户端提交的答案：形状、上限、幂等键。
pub fn validate_answer_input(input: &InboxAnswerInput) -> Result<(), InboxInvalid> {
    if input.answer_id.is_empty()
        || input.answer_id.len() > 64
        || !input
            .answer_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(Invalid::field("answer_id", "要 1–64 个字母、数字、_ 或 -"));
    }
    many("answers", input.answers.len(), MAX_INBOX_QUESTIONS)?;
    if let Some(comment) = &input.comment {
        long("comment", comment, MAX_INBOX_COMMENT_CHARS)?;
    }
    if let Some(action_id) = &input.action_id {
        validate_action_id("action_id", action_id)?;
    }
    let mut ids = HashSet::new();
    for answer in &input.answers {
        present("answers.question_id", &answer.question_id)?;
        if !ids.insert(answer.question_id.as_str()) {
            return Err(InboxInvalid::Duplicate {
                field: "answers".to_owned(),
                id: answer.question_id.clone(),
            });
        }
        for selected in &answer.selected {
            present("answers.selected", selected)?;
        }
        if let Some(other) = &answer.other {
            long("answers.other", other, MAX_INBOX_OTHER_CHARS)?;
        }
        if let Some(note) = &answer.note {
            long("answers.note", note, MAX_INBOX_NOTE_CHARS)?;
        }
    }
    Ok(())
}

/// 校验答案和条目对得上：题目 id、选项 id、单选/多选、必答、动作 id。
pub fn validate_answer(item: &InboxItem, input: &InboxAnswerInput) -> Result<(), InboxInvalid> {
    validate_answer_input(input)?;
    match &item.payload {
        InboxPayload::Choice(payload) => validate_choice_answer(payload, input),
        InboxPayload::Page(payload) => validate_action_answer(&payload.actions, input, true),
        InboxPayload::Recap(payload) => {
            if payload.question.is_some() {
                validate_action_answer(&payload.actions, input, true)
            } else {
                validate_action_answer(&payload.actions, input, false)
            }
        }
        InboxPayload::Notice(payload) => {
            if let Some(action) = &payload.action {
                validate_action_answer(std::slice::from_ref(action), input, false)
            } else if input.action == InboxAnswerAction::Acknowledge {
                Ok(())
            } else {
                Err(Invalid::field("action", "这条通知只有知道了"))
            }
        }
        InboxPayload::Confirm(payload) => validate_confirm_answer(payload, input),
        InboxPayload::Other(_) => Ok(()),
    }
}

fn validate_choice_answer(
    payload: &ChoicePayload,
    input: &InboxAnswerInput,
) -> Result<(), InboxInvalid> {
    match input.action {
        InboxAnswerAction::Decline => return Ok(()),
        InboxAnswerAction::Submit => {}
        other => {
            return Err(Invalid::field(
                "action",
                &format!("问题卡只收 submit 或 decline，收到 {other:?}"),
            ));
        }
    }
    let by_id: std::collections::HashMap<&str, &InboxQuestion> = payload
        .questions
        .iter()
        .map(|question| (question.id.as_str(), question))
        .collect();
    for answer in &input.answers {
        let Some(question) = by_id.get(answer.question_id.as_str()) else {
            return Err(Invalid::field("answers", "题目不在这张卡里"));
        };
        if answer.skipped {
            if !question.skippable {
                return Err(Invalid::field("answers.skipped", "这题不能跳过"));
            }
            continue;
        }
        if answer.note.is_some() && !question.note_enabled {
            return Err(Invalid::field("answers.note", "这题不收备注"));
        }
        if question.options.is_empty() {
            if !answer.selected.is_empty() && !question.multi_select && answer.selected.len() > 1 {
                return Err(Invalid::field("answers.selected", "单选最多一项"));
            }
            continue;
        }
        if answer.selected.is_empty() && answer.other.is_none() {
            return Err(Invalid::field("answers", "必答的题要选一项或写“其他”"));
        }
        if !question.multi_select && answer.selected.len() > 1 {
            return Err(Invalid::field("answers.selected", "单选最多一项"));
        }
        let options: HashSet<&str> = question.options.iter().map(|o| o.id.as_str()).collect();
        for selected in &answer.selected {
            if !options.contains(selected.as_str()) {
                return Err(Invalid::field("answers.selected", "选项不在这道题里"));
            }
        }
    }
    for question in &payload.questions {
        if question.skippable {
            continue;
        }
        if !input.answers.iter().any(|a| a.question_id == question.id) {
            return Err(Invalid::field("answers", "必答的题没有答"));
        }
    }
    Ok(())
}

fn validate_action_answer(
    actions: &[InboxAction],
    input: &InboxAnswerInput,
    require_action: bool,
) -> Result<(), InboxInvalid> {
    if input.action == InboxAnswerAction::Acknowledge {
        if require_action && !actions.is_empty() {
            return Err(Invalid::field("action", "有动作时不能只知道了"));
        }
        return Ok(());
    }
    match input.action {
        InboxAnswerAction::Action | InboxAnswerAction::Submit => {}
        other => {
            return Err(Invalid::field(
                "action",
                &format!("这一条只收 action 或 acknowledge，收到 {other:?}"),
            ));
        }
    }
    let Some(action_id) = &input.action_id else {
        return Err(Invalid::field("action_id", "要带 action_id"));
    };
    let Some(action) = actions.iter().find(|action| &action.id == action_id) else {
        return Err(Invalid::field("action_id", "动作不在这条里"));
    };
    if action.requires_comment
        && input
            .comment
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return Err(Invalid::field("comment", "这个动作要写备注"));
    }
    Ok(())
}

fn validate_confirm_answer(
    payload: &ConfirmPayload,
    input: &InboxAnswerInput,
) -> Result<(), InboxInvalid> {
    let needed = match input.action {
        InboxAnswerAction::Approve => Some(ConfirmAction::Approve),
        InboxAnswerAction::Deny => Some(ConfirmAction::Deny),
        InboxAnswerAction::Session => Some(ConfirmAction::Session),
        InboxAnswerAction::Always => Some(ConfirmAction::Always),
        other => {
            return Err(Invalid::field(
                "action",
                &format!("确认只收 approve、deny、session、always，收到 {other:?}"),
            ));
        }
    };
    if !payload
        .choices
        .iter()
        .any(|choice| Some(choice.action) == needed)
    {
        return Err(Invalid::field("action", "发送方没给这一档"));
    }
    Ok(())
}

/// 判断一条条目是不是还没处理、可以答的结构。
#[must_use]
pub fn answerable(item: &InboxItem) -> bool {
    matches!(item.status, InboxStatus::Pending) && !matches!(item.payload, InboxPayload::Other(_))
}

struct Invalid;

impl Invalid {
    fn field(field: &str, reason: &str) -> InboxInvalid {
        InboxInvalid::Invalid {
            field: field.to_owned(),
            value: String::new(),
            reason: reason.to_owned(),
        }
    }
}

/// 把一道题的答案里的选项排成题干里的顺序（界面复现用；缺的不补）。
#[must_use]
pub fn ordered_selection(question: &InboxQuestion, selected: &[String]) -> Vec<String> {
    question
        .options
        .iter()
        .filter(|option| selected.iter().any(|id| id == &option.id))
        .map(|option| option.id.clone())
        .collect()
}
