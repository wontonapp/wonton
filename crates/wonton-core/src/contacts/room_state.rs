//! 联系人房间的建立进度：每台服务器的两种退避（建房/传输失败的重退避，邀请重发的轻退避）
//! 和最后一次失败。失败状态经 [`RoomBackoff::last_error`] 进联系人行的视图；不落盘，
//! 重启后由收件循环重新算出来。

use std::collections::BTreeMap;
use std::time::Duration;

use wonton_protocol::ContactError;

/// 建房/邀请的按服务器退避。两种节奏：建房和传输失败的重退避，邀请重发的轻退避——成员检查
/// 不受任何退避约束（一次查询而已，还是唯一能把“还没通”清掉的东西）。
pub(super) struct RoomBackoff {
    attempts: BTreeMap<String, u32>,
    next_at: BTreeMap<String, tokio::time::Instant>,
    invite_attempts: BTreeMap<String, u32>,
    invite_at: BTreeMap<String, tokio::time::Instant>,
    last_error: BTreeMap<String, ContactError>,
    first: Duration,
    max: Duration,
}

impl RoomBackoff {
    pub(super) fn new(first: Duration, max: Duration) -> Self {
        Self {
            attempts: BTreeMap::new(),
            next_at: BTreeMap::new(),
            invite_attempts: BTreeMap::new(),
            invite_at: BTreeMap::new(),
            last_error: BTreeMap::new(),
            first,
            max,
        }
    }

    /// 现在能建房吗。被动等待（字节序较大的一方等邀请）不算失败，不进退避。
    pub(super) fn ready(&self, server: &str) -> bool {
        self.next_at
            .get(server)
            .is_none_or(|at| *at <= tokio::time::Instant::now())
    }

    /// 现在能再发一次邀请吗（M_FORBIDDEN 的“已在邀请里”也算发过）。
    pub(super) fn invite_ready(&self, server: &str) -> bool {
        self.invite_at
            .get(server)
            .is_none_or(|at| *at <= tokio::time::Instant::now())
    }

    pub(super) fn note_failure(&mut self, server: &str, error: ContactError) {
        let attempts = self.attempts.entry(server.to_owned()).or_default();
        let delay = self.first * (1u32 << (*attempts).min(16));
        *attempts += 1;
        self.next_at.insert(
            server.to_owned(),
            tokio::time::Instant::now() + delay.min(self.max),
        );
        self.note_invite(server);
        self.last_error.insert(server.to_owned(), error);
    }

    /// 邀请刚尝试过（成了或被拒）：轻退避后再试，重发不要太密。
    pub(super) fn note_invite(&mut self, server: &str) {
        let attempts = self.invite_attempts.entry(server.to_owned()).or_default();
        let delay = self.first * (1u32 << (*attempts).min(16));
        *attempts += 1;
        self.invite_at.insert(
            server.to_owned(),
            tokio::time::Instant::now() + delay.min(self.max),
        );
    }

    /// 邀请已发出、对方还没进房：如实记“没通”，不动退避。
    pub(super) fn note_pending(&mut self, server: &str) {
        self.last_error
            .insert(server.to_owned(), ContactError::PeerUnreachable);
    }

    pub(super) fn note_success(&mut self, server: &str) {
        self.attempts.remove(server);
        self.next_at.remove(server);
        self.invite_attempts.remove(server);
        self.invite_at.remove(server);
        self.last_error.remove(server);
    }

    /// 联系人行里如实显示的房间失败（没有就是通了或在被动等待）。
    pub(super) fn last_error(&self, server: &str) -> Option<ContactError> {
        self.last_error.get(server).cloned()
    }
}
