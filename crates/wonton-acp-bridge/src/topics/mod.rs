//! 话题会话（ADR-0058）第 1 步：工作板、改文件表、动态摘要。
//!
//! 这一层只管记录和生成文字，不碰 Matrix 也不碰 ACP 连接；接线（按话题分会话、发
//! notice、权限排队卡片、set_mode）在第 2 步。一个桥实例只服务一个房间、一个房间一张
//! 工作板，所以 [`Topics`] 不带房间维度；将来一桥多房间时按房间各建一个 [`Topics`]。
//!
//! 每个方法该在第 2 步的哪里调用，见各自 doc 注释。时间一律由调用方传入
//! （桥传 `SystemTime::now()`，测试传固定值），模块不读系统时钟；除查询外的每次
//! 变动都写盘，写盘失败向上报 `Error::Upstream`，不静默吞掉。

mod board;
mod claims;
mod digest;
mod limits;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::{path::Path, time::SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use board::{Board, NAME_MAX_CHARS, Row, SUMMARY_LINES, Status, TEXT_MAX_CHARS};

/// 会话键：主对话是 [`SessionKey::Main`]，话题用它的根消息事件 ID。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKey {
    Main,
    /// 话题根消息的事件 ID。
    Topic(String),
}

impl SessionKey {
    pub(crate) fn is_topic(&self) -> bool {
        matches!(self, SessionKey::Topic(_))
    }
}

/// [`Topics::turn_started`] 的结果：话题直接开始，还是因为活跃话题已满在排队。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnStart {
    Started,
    Queued,
}

/// 两个会话先后都改了同一个文件（事后发现的冲突，ADR-0058：全自动模式拦不住，
/// 只能事后提醒），第 2 步拿来在两边各发一句。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditConflict {
    pub other: SessionKey,
    pub path: String,
}

/// 话题会话的门面：工作板、改文件表、上限和文字生成都从这里进出。
#[derive(Debug)]
pub struct Topics {
    board: Board,
}

impl Topics {
    /// 打开（没有就新建）桥数据目录下的话题工作板文件。`now` 用于给重启前没结束的
    /// 轮次补上改文件记录的保留期。第 2 步在桥启动时调用一次。
    pub async fn open(path: impl AsRef<Path>, now: SystemTime) -> Result<Self> {
        let mut board = Board::load(path).await?;
        board.seal_all_claims(stamp(now));
        Ok(Self { board })
    }

    /// 每轮开始（主对话和话题都一样）在把这句话发给 Agent 之前调。先 [`Topics::digest_for`]
    /// 拿动态拼提示词，再调这里登记“在做什么”。新话题由此入场；活跃话题已满时返回
    /// [`TurnStart::Queued`]，这时不要发提示词，把首条消息留着，等
    /// [`Topics::poll_admission`] 放行后再发。
    pub async fn turn_started(
        &mut self,
        key: &SessionKey,
        prompt: &str,
        now: SystemTime,
    ) -> Result<TurnStart> {
        let now = stamp(now);
        let base = self.board.seq();
        let admit = !key.is_topic() || limits::can_admit(&self.board);
        let started = if let Some(row) = self.board.row_mut(key) {
            row.doing = digest::clip(prompt, TEXT_MAX_CHARS);
            row.last_activity = now;
            row.files.clear();
            // 排队中不会有空位轮到它插队；已结束的话题又来消息，按入场规则重新排。
            let started = match row.status {
                Status::Queued => false,
                Status::Finished => admit,
                _ => true,
            };
            row.status = if started {
                Status::Working
            } else {
                Status::Queued
            };
            row.pending_prompt = (!started).then(|| prompt.to_owned());
            row.last_seen_seq = base;
            started
        } else {
            let status = if admit {
                Status::Working
            } else {
                Status::Queued
            };
            let name = if key.is_topic() {
                digest::clip_plain(prompt, NAME_MAX_CHARS)
            } else {
                "主对话".to_owned()
            };
            self.board.add_row(Row {
                key: key.clone(),
                name,
                doing: digest::clip(prompt, TEXT_MAX_CHARS),
                status,
                summary: Vec::new(),
                files: Vec::new(),
                last_seen_seq: base,
                last_activity: now,
                pending_prompt: (!admit).then(|| prompt.to_owned()),
            });
            admit
        };
        self.board.persist().await?;
        Ok(if started {
            TurnStart::Started
        } else {
            TurnStart::Queued
        })
    }

    /// 收到 ACP `tool_call`/`tool_call_update` 通知（`AgentNotice::Update`）时调：
    /// kind 为 edit/delete/move 的 locations 记进改文件表；另一个会话正在改同一文件时
    /// 返回事后冲突，第 2 步拿来在两边各发一句。execute 不记文件，只在工作板上记一笔。
    /// 其他更新原样忽略，返回 `None`。
    pub async fn tool_call(
        &mut self,
        key: &SessionKey,
        update: &Value,
        now: SystemTime,
    ) -> Result<Option<EditConflict>> {
        let now = stamp(now);
        let (files, executed) = match claims::parse_tool_call(update) {
            claims::ToolEdit::Files(paths) if !paths.is_empty() => (Some(paths), None),
            claims::ToolEdit::Executed(title) => (None, Some(title)),
            _ => return Ok(None),
        };
        let name = self
            .board
            .row(key)
            .map(|row| row.name.clone())
            .unwrap_or_default();
        let conflict = files
            .as_ref()
            .and_then(|paths| claims::conflict_on(self.board.claims(), key, paths, now));
        if let Some(paths) = &files {
            if let Some((other, path)) = &conflict {
                let other_name = self
                    .board
                    .row(other)
                    .map(|row| row.name.clone())
                    .unwrap_or_default();
                self.board.log_change(
                    key.clone(),
                    digest::conflict_text(&other_name, &name, path),
                    true,
                );
            }
            self.board.record_claims(key, paths, now);
            if let Some(row) = self.board.row_mut(key) {
                for path in paths {
                    if !row.files.contains(path) {
                        row.files.push(path.clone());
                    }
                }
            }
        }
        if let Some(title) = executed {
            self.board
                .log_change(key.clone(), digest::executed_text(&name, &title), false);
        }
        self.board.persist().await?;
        Ok(conflict.map(|(other, path)| EditConflict { other, path }))
    }

    /// Agent 请求改文件权限（`session/request_permission` 且工具要写文件）时调：把该
    /// 会话标成“等你确认”，并返回正在改这些路径的另一个会话；有冲突时第 2 步弹排队
    /// 卡片（默认选项是等它做完再改），没有冲突就照常弹确认卡片。
    pub async fn permission_conflict(
        &mut self,
        key: &SessionKey,
        paths: &[String],
        now: SystemTime,
    ) -> Result<Option<SessionKey>> {
        let now = stamp(now);
        let mut changed = false;
        if let Some(row) = self.board.row_mut(key)
            && row.status != Status::WaitingApproval
        {
            row.status = Status::WaitingApproval;
            changed = true;
        }
        let conflict = claims::conflict_on(self.board.claims(), key, paths, now);
        if changed {
            self.board.persist().await?;
        }
        Ok(conflict.map(|(other, _)| other))
    }

    /// 权限卡片有了结果、桥答复 ACP 之后调：把会话从“等你确认”放回“进行中”。
    pub async fn permission_settled(&mut self, key: &SessionKey) -> Result<()> {
        let changed = match self.board.row_mut(key) {
            Some(row) if row.status == Status::WaitingApproval => {
                row.status = Status::Working;
                true
            }
            _ => false,
        };
        if changed {
            self.board.persist().await?;
        }
        Ok(())
    }

    /// 一轮结束（ACP prompt 调用返回）时调：记结论、给改文件记录盖章（结束后再保留
    /// 10 分钟）、状态放回“空闲”。话题会话返回主对话里那行 notice 的文字；主对话
    /// 返回 `None`（它的结论由话题的下一轮动态带上，不往话题里发）。排队中的话题
    /// 没跑过轮，接到这个调用时忽略。
    pub async fn turn_finished(
        &mut self,
        key: &SessionKey,
        last_reply: &str,
        now: SystemTime,
    ) -> Result<Option<String>> {
        let now = stamp(now);
        let Some(row) = self.board.row_mut(key) else {
            return Ok(None);
        };
        if row.status == Status::Queued {
            return Ok(None);
        }
        row.summary = summary_lines(last_reply);
        row.status = Status::Idle;
        row.last_activity = now;
        let name = row.name.clone();
        let files = row.files.clone();
        let summary = row.summary.clone();
        self.board.seal_claims_of(key, now);
        if !summary.is_empty() || !files.is_empty() {
            self.board.log_change(
                key.clone(),
                digest::turn_done_text(&name, &summary, &files),
                false,
            );
        }
        let notice = key
            .is_topic()
            .then(|| digest::finish_notice(&name, files.len()));
        self.board.persist().await?;
        Ok(notice)
    }

    /// 每轮开头、把用户这句话发给 Agent 之前调（在 [`Topics::turn_started`] 之前，
    /// 它读的是上一轮记下的序号）：其他会话从上一轮开始到现在的工作板变化。
    /// 没有变化返回 `None`，第 2 步就不用拼动态。
    pub fn digest_for(&self, key: &SessionKey) -> Option<String> {
        digest::digest(&self.board, key)
    }

    /// 话题第一条消息到达时调（在 [`Topics::turn_started`] 之前）：话题开局说明。
    /// `root` 是话题起点那条消息，`preceding` 是它之前的主对话消息，按时间先后；
    /// 超过 6 条只带最近的 6 条。排队的话题晋升后用它重新拼一次再发。
    pub fn opening_for(&self, key: &SessionKey, root: &str, preceding: &[String]) -> String {
        digest::opening(&self.board, key, root, preceding)
    }

    /// 用户在话题里发 `/wonton end` 时调：把话题标成已结束。返回是否确实结束了一个
    /// 开着的话题（主对话或未知会话返回 `false`，第 2 步照实回话）。
    pub async fn end(&mut self, key: &SessionKey, now: SystemTime) -> Result<bool> {
        if limits::end(&mut self.board, key, stamp(now)) {
            self.board.persist().await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// 桥的空闲定时器里调：把空闲超过 30 分钟的话题判为结束，返回它们的会话键，
    /// 第 2 步据此关 Agent 会话（支持 `session/close` 就关）并在话题里告知。
    pub async fn sweep(&mut self, now: SystemTime) -> Result<Vec<SessionKey>> {
        let ended = limits::sweep(&mut self.board, stamp(now));
        if !ended.is_empty() {
            self.board.persist().await?;
        }
        Ok(ended)
    }

    /// 有话题结束之后调（[`Topics::end`] 返回 true、[`Topics::sweep`] 非空）：名额有
    /// 空就放最老的排队话题进来，返回它的会话键和首条消息；第 2 步拼上开局说明和
    /// 动态再发提示词。没有返回值就继续等。
    pub async fn poll_admission(
        &mut self,
        now: SystemTime,
    ) -> Result<Option<(SessionKey, String)>> {
        let promoted = limits::poll_admission(&mut self.board, stamp(now));
        if promoted.is_some() {
            self.board.persist().await?;
        }
        Ok(promoted)
    }
}

/// 结论取 Agent 这轮最后回复的前 3 行（跳过空行，每行截短）。
fn summary_lines(reply: &str) -> Vec<String> {
    reply
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(SUMMARY_LINES)
        .map(|line| digest::clip(line, TEXT_MAX_CHARS))
        .collect()
}

fn stamp(now: SystemTime) -> u64 {
    now.duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
