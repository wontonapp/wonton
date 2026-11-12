use wonton_protocol::{CoreError, SessionListViewModel, SessionRow};

/// 会话状态和处理操作的规则。纯同步逻辑，不知道 UniFFI 和订阅的存在。
#[derive(Debug, Default)]
pub(crate) struct SessionStore {
    /// 最新的在前。
    sessions: Vec<Session>,
    selected_id: Option<String>,
    created_count: u64,
}

#[derive(Debug)]
struct Session {
    id: String,
    title: String,
}

/// 会话操作。返回错误时状态保持不变。
impl SessionStore {
    pub(crate) fn view_model(&self) -> SessionListViewModel {
        let sessions = self
            .sessions
            .iter()
            .map(|session| SessionRow {
                id: session.id.clone(),
                title: session.title.clone(),
                is_selected: self.selected_id.as_deref() == Some(session.id.as_str()),
            })
            .collect();
        SessionListViewModel { sessions }
    }

    pub(crate) fn create(&mut self, title: &str) -> Result<(), CoreError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(invalid_input("会话标题不能为空"));
        }
        self.created_count += 1;
        let id = format!("session-{}", self.created_count);
        self.sessions.insert(
            0,
            Session {
                id: id.clone(),
                title: title.to_owned(),
            },
        );
        self.selected_id = Some(id);
        Ok(())
    }

    pub(crate) fn select(&mut self, session_id: String) -> Result<(), CoreError> {
        self.position(&session_id)?;
        self.selected_id = Some(session_id);
        Ok(())
    }

    pub(crate) fn delete(&mut self, session_id: &str) -> Result<(), CoreError> {
        let index = self.position(session_id)?;
        self.sessions.remove(index);
        if self.selected_id.as_deref() == Some(session_id) {
            self.selected_id = None;
        }
        Ok(())
    }

    fn position(&self, session_id: &str) -> Result<usize, CoreError> {
        self.sessions
            .iter()
            .position(|session| session.id == session_id)
            .ok_or_else(|| invalid_input(&format!("会话不存在：{session_id}")))
    }
}

fn invalid_input(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}
