//! Bot 资料以账号自己的临时会话读写，状态头像使用 MSC4133 自定义资料字段。

use hyper::Method;
use serde_json::{Value, json};
use wonton_protocol::{AVATARS_FIELD, StateAvatars};

use super::{Matrix, MatrixError, Session};

impl Matrix {
    /// 只登录已有的本站账号，避免 JWT 登录把拼错的 ID 变成新账号。
    pub async fn as_bot<T>(
        &self,
        user_id: &str,
        work: impl AsyncFnOnce(&Session<'_>) -> Result<T, MatrixError>,
    ) -> Result<T, MatrixError> {
        let (localpart, server) = user_id
            .strip_prefix('@')
            .and_then(|id| id.split_once(':'))
            .filter(|(localpart, _)| !localpart.is_empty())
            .ok_or_else(|| MatrixError::Invalid(format!("Bot 的用户 ID `{user_id}` 不合法")))?;
        if server != self.server_name() {
            return Err(MatrixError::Invalid("Bot 不属于本站".into()));
        }
        if !self.user_exists(user_id).await? {
            return Err(MatrixError::Invalid(format!("Bot 账号 `{user_id}` 不存在")));
        }
        let login = self
            .login(localpart, "Wonton 核心（临时修改 Bot 资料）")
            .await?;
        let session = self.session(&login);
        let result = work(&session).await;
        let logout = session.logout().await;
        if let Err(error) = &logout {
            tracing::error!(user_id, %error, "退出 Bot 资料临时会话失败");
        }
        result.and_then(|value| logout.map(|()| value))
    }

    /// 经核心会话读取头像；多读一个字节供媒体检查区分文件过大。
    pub async fn download_media(&self, url: &str) -> Result<Vec<u8>, MatrixError> {
        let (server, id) = mxc_parts(url)?;
        self.api.download(&self.admin_token, server, id).await
    }
}

impl Session<'_> {
    pub async fn avatar_url(&self) -> Result<Option<String>, MatrixError> {
        match self
            .json(
                Method::GET,
                &self.profile_path("avatar_url"),
                None,
                "读头像",
            )
            .await
        {
            Ok(value) => Ok(profile_string(&value, "avatar_url")?.filter(|url| !url.is_empty())),
            Err(error) if error.errcode() == "M_NOT_FOUND" => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn set_avatar_url(&self, url: Option<&str>) -> Result<(), MatrixError> {
        // Tuwunel v1.9.4 requires a string; null is M_BAD_JSON, empty removes the avatar.
        let body = json!({"avatar_url": url.unwrap_or("")});
        self.json(
            Method::PUT,
            &self.profile_path("avatar_url"),
            Some(&body),
            "设置头像",
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn has_avatars(&self) -> Result<bool, MatrixError> {
        match self
            .json(
                Method::GET,
                &self.profile_path(AVATARS_FIELD),
                None,
                "查状态头像字段",
            )
            .await
        {
            Ok(value) if value.get(AVATARS_FIELD).is_some() => Ok(true),
            Ok(_) => Err(MatrixError::Invalid("状态头像响应缺少字段".into())),
            Err(error) if error.errcode() == "M_NOT_FOUND" => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// `None` 是字段不存在；空表仍然是 `Some`，不能再按清单补默认素材。
    pub async fn avatars(&self) -> Result<Option<StateAvatars>, MatrixError> {
        let value = match self
            .json(
                Method::GET,
                &self.profile_path(AVATARS_FIELD),
                None,
                "读状态头像",
            )
            .await
        {
            Ok(value) => value,
            Err(error) if error.errcode() == "M_NOT_FOUND" => return Ok(None),
            Err(error) => return Err(error),
        };
        let field = value
            .get(AVATARS_FIELD)
            .ok_or_else(|| MatrixError::Invalid("状态头像响应缺少字段".into()))?;
        if !field.get("states").is_some_and(Value::is_object) {
            return Err(MatrixError::Invalid("状态头像响应缺少 states 对象".into()));
        }
        serde_json::from_value(field.clone())
            .map(Some)
            .map_err(|error| MatrixError::Invalid(format!("状态头像响应不对：{error}")))
    }

    pub async fn set_avatars(&self, avatars: &StateAvatars) -> Result<(), MatrixError> {
        let body = json!({AVATARS_FIELD: avatars});
        self.json(
            Method::PUT,
            &self.profile_path(AVATARS_FIELD),
            Some(&body),
            "设置状态头像",
        )
        .await
        .map(|_| ())
    }

    pub async fn delete_avatars(&self) -> Result<(), MatrixError> {
        self.json(
            Method::DELETE,
            &self.profile_path(AVATARS_FIELD),
            None,
            "删除状态头像字段",
        )
        .await
        .map(|_| ())
    }

    pub async fn upload_media(
        &self,
        content_type: &str,
        filename: &str,
        data: Vec<u8>,
    ) -> Result<String, MatrixError> {
        self.api
            .upload(&self.token, content_type, filename, data)
            .await
    }
}

pub(super) fn profile_string(value: &Value, field: &str) -> Result<Option<String>, MatrixError> {
    if !value.is_object() {
        return Err(MatrixError::Invalid("资料响应不是对象".into()));
    }
    match value.get(field) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        None | Some(Value::Null) => Ok(None),
        _ => Err(MatrixError::Invalid(format!("资料响应的 {field} 字段不对"))),
    }
}

fn mxc_parts(url: &str) -> Result<(&str, &str), MatrixError> {
    let invalid = || MatrixError::Invalid(format!("媒体地址 `{url}` 不是合法的 mxc 地址"));
    let (server, id) = url
        .strip_prefix("mxc://")
        .and_then(|rest| rest.split_once('/'))
        .ok_or_else(invalid)?;
    // 不重解释百分号编码；媒体 ID 只能是一个不转义的 URI 路径段。
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
    {
        return Err(invalid());
    }
    let authority: hyper::http::uri::Authority = server.parse().map_err(|_| invalid())?;
    if server.is_empty()
        || server.contains('@')
        || authority.host().is_empty()
        || server.bytes().any(|byte| {
            !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
        })
        || (!server.ends_with(']')
            && server
                .rsplit_once(':')
                .is_some_and(|(_, port)| port.parse::<u16>().is_err()))
    {
        return Err(invalid());
    }
    Ok((server, id))
}
