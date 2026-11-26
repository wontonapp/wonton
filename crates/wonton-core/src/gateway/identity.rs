//! 请求是谁：令牌、设备记录和 Cookie 会话，以及按身份取资料、站点源和 Cookie 属性。

use hyper::HeaderMap;
use hyper::header::{ACCEPT_LANGUAGE, AUTHORIZATION};
use wonton_protocol::{MemberStatus, Permissions};

use crate::owner::Owner;
use crate::secret::constant_time_eq;

use super::*;

impl Gateway {
    pub(super) fn check_owner_token(&self, token: &str) -> bool {
        constant_time_eq(token.trim(), &self.owner_token)
    }

    /// 访问令牌是谁（Matrix 用户 ID）：`whoami`，按令牌的哈希缓存（和发送限制共用一份，见
    /// [`send_limits::SendLimiter`]）。没接入 Matrix 或认不出时是 `None`。
    pub(crate) async fn bot_user_of_token(&self, token: &str) -> Option<String> {
        match self.bot_identity(token).await {
            Identity::Bot(user) => Some(user),
            Identity::Unknown | Identity::Unavailable => None,
        }
    }

    /// 同 [`Self::bot_user_of_token`]，但分得开令牌失效和问不到 Tuwunel。
    pub(crate) async fn bot_identity(&self, token: &str) -> Identity {
        self.send_limits
            .identify(
                token,
                send_limits::token_hash(token),
                self.matrix.as_deref(),
            )
            .await
    }

    /// 经隧道接入的已配对设备是谁（ADR-0045）：设备记录里写了家人就是他，没写是主人。吊销后立即不再认它，
    /// 哪怕请求还在一条没来得及断开的连接上。
    pub(super) fn device_person(&self, peer: Option<TunnelPeer>) -> Option<Person> {
        let TunnelPeer(id) = peer?;
        let device = self.tunnel.devices.get(&id)?;
        let Some(username) = device.user else {
            return Some(Person::owner(self.owner.username()));
        };
        // 失败即停：设备记的家人必须在记录里、还没被移除。记录对不上时按认不出处理，不编一个身份放行。
        match self.members.get(&username) {
            Some(record)
                if record.status != MemberStatus::Removed
                    && !self.members.pending_endpoint(id, &username) =>
            {
                Some(Person::member(&record.username))
            }
            _ => {
                tracing::warn!(
                    device = %id.fmt_short(),
                    member = %username,
                    "设备的家人不在记录里、尚未批准或已移除，拒绝这台设备的请求"
                );
                None
            }
        }
    }

    /// 经隧道接入的已配对设备属于谁：用户名（家人）或主人。
    pub(super) fn device_user(&self, peer: Option<TunnelPeer>) -> Option<String> {
        self.device_person(peer).map(|person| person.username)
    }

    /// 请求认得出是谁时，把它的 `Accept-Language` 记在这个人名下（ADR-0043）：语言按人给，主人和家人各记各的。
    /// 认不出人就不记。
    pub(crate) fn note_languages(&self, peer: Option<TunnelPeer>, headers: &HeaderMap) {
        let person = self.device_person(peer).or_else(|| {
            let token = headers
                .get(AUTHORIZATION)?
                .to_str()
                .ok()?
                .strip_prefix("Bearer ")?;
            self.check_owner_token(token)
                .then(|| Person::owner(self.owner.username()))
        });
        let Some(person) = person else { return };
        let accept = headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok());
        self.plugins.note_person_languages(&person.username, accept);
    }

    /// 核心域名上的当前人：经隧道的已配对设备，或者局域网浏览器的核心会话（主人令牌登录，是主人）。
    pub(super) fn core_person(
        &self,
        peer: Option<TunnelPeer>,
        headers: &HeaderMap,
    ) -> Option<Person> {
        self.device_person(peer).or_else(|| {
            session::cookies(headers, CORE_COOKIE).find_map(|sid| self.sessions.core_user(sid))
        })
    }

    /// 插件域上的当前人：已配对设备或本域插件会话。来源隔离或受限 OIDC 回调允许后才使用。
    pub(super) fn plugin_person(&self, host: &str, req: &Request) -> Option<Person> {
        self.device_person(req.extensions().get().copied())
            .or_else(|| {
                session::cookies(req.headers(), APP_COOKIE)
                    .find_map(|sid| self.sessions.app_user(sid, host))
            })
    }

    /// 从 `owner.json` / `members.json` 取本人资料；插件出口只用用户名派生内部邮箱，不公开真实邮箱。
    /// 复用 [`Owner`] 值类型；家人的记录找不到就报错，不编一个身份出来。
    pub(crate) fn identity_of(&self, person: &Person) -> Result<Owner, MissingMember> {
        if person.is_owner() {
            return Ok(self.owner.owner());
        }
        if person.role == Role::Friend {
            let record = self
                .sharing
                .as_ref()
                .and_then(|sharing| sharing.friends.find_username(&person.username))
                .ok_or_else(|| MissingMember(person.username.clone()))?;
            return Ok(Owner {
                username: record.username,
                display_name: record.display_name.unwrap_or_default(),
                email: None,
            });
        }
        let record = self
            .members
            .get(&person.username)
            .ok_or_else(|| MissingMember(person.username.clone()))?;
        Ok(Owner {
            username: record.username,
            display_name: record.display_name,
            email: record.email,
        })
    }

    /// 这台设备的人在 `GET ME_PATH` 里看到的自己（ADR-0045 的 `Me`）。家人的记录找不到了就报错：
    /// 设备刚被移除时会这样，不编一个身份出来。
    pub(crate) fn me(&self, person: &Person) -> Result<wonton_protocol::Me, MissingMember> {
        let user_id = matrix_user_id(self, &person.username);
        if person.is_owner() {
            let profile = self.owner.profile();
            Ok(wonton_protocol::Me {
                user_id,
                username: profile.username,
                is_owner: true,
                role_id: None,
                role_name: None,
                permissions: None,
                display_name: profile.display_name,
                email: profile.email,
                avatar: None,
            })
        } else {
            let record = self
                .members
                .get(&person.username)
                .ok_or_else(|| MissingMember(person.username.clone()))?;
            if record.status == MemberStatus::Removed {
                return Err(MissingMember(person.username.clone()));
            }
            // 角色记录读不到时按最严的权限报，不冒充能给：`role_id` 照实给出记录里那条。
            let role = self.roles.get(&record.role_id);
            Ok(wonton_protocol::Me {
                user_id,
                username: record.username,
                is_owner: false,
                role_name: role
                    .as_ref()
                    .map(|role| role.name.trim())
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                permissions: Some(
                    role.as_ref()
                        .map_or_else(Permissions::strictest, |role| role.permissions.clone()),
                ),
                role_id: Some(record.role_id),
                display_name: Some(record.display_name),
                email: record.email,
                avatar: None,
            })
        }
    }

    pub(super) fn secure_cookies(&self) -> bool {
        self.public.scheme == "https"
    }

    pub(super) fn core_origin(&self) -> String {
        self.public.origin(self.public.core_host())
    }
}
