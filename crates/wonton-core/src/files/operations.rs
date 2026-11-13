use super::record::{Accessor, Lookup, Origin};
use super::share_fs::Deleted;
use super::{DELETED_UNSYNCED, Failure, FileEvent, Files, share_failure};
use crate::l10n;
use crate::plugin::{languages::Languages, manager::now_secs};
use std::sync::Arc;
use wonton_protocol::{FileFailure, FileId};

impl Files {
    /// Owner-only core callers retain the real owner's identity, never the current device user.
    pub async fn grant_room(
        self: &Arc<Self>,
        id: &FileId,
        room: &str,
        langs: &Languages,
    ) -> Result<Vec<String>, Failure> {
        self.grant_room_for(Accessor::Owner, id, room, langs).await
    }

    pub async fn revoke_bot(
        self: &Arc<Self>,
        id: &FileId,
        bot: &str,
        langs: &Languages,
    ) -> Result<(), Failure> {
        self.revoke_bot_for(Accessor::Owner, id, bot, langs).await
    }

    pub async fn send_to_devices(
        self: &Arc<Self>,
        id: &FileId,
        langs: &Languages,
    ) -> Result<(), Failure> {
        self.send_to_devices_for(Accessor::Owner, id, langs).await
    }

    pub async fn delete(
        self: &Arc<Self>,
        id: &FileId,
        device: String,
        langs: &Languages,
    ) -> Result<(), Failure> {
        self.delete_for(Accessor::Owner, id, device, langs).await
    }

    /// 把这个房间的 Bot 加进访问范围，返回加了哪些。房间里没有 Bot 时也会清掉过期：App 授权之后马上发链接，这时链接就算发出去了。
    /// 和到期清理排同一个队：清理已经取了过期名单也不会在授权之后把文件删掉。
    pub async fn grant_room_for(
        self: &Arc<Self>,
        who: Accessor,
        id: &FileId,
        room: &str,
        langs: &Languages,
    ) -> Result<Vec<String>, Failure> {
        let (task, room, langs) = (id.clone(), room.to_owned(), langs.clone());
        let turn = self.uploads.turn(id).await;
        let bots = self
            .blocking(move |files| {
                files.authorize(&task, &who, &langs)?;
                // 主人已经在范围里，`grant` 也负责清掉过期时间（ADR-0020 的“生命周期”）。
                files
                    .registry
                    .grant(&task, who, now_secs())?
                    .ok_or_else(|| Failure::unknown(&langs))?;
                let bots = files.host.bots_in_room(&room)?;
                for bot in &bots {
                    files
                        .registry
                        .grant_by_owner(&task, bot, now_secs())?
                        .ok_or_else(|| Failure::unknown(&langs))?;
                }
                Ok(bots)
            })
            .await;
        drop(turn);
        bots
    }

    /// 单独收回一个 Bot 的访问：来源插件的授权收回时，主人亲手授权过的 Bot 不受影响，要靠这个。这个 Bot 再经工具找到同一个文件时，
    /// 按来源插件的授权算。
    pub async fn revoke_bot_for(
        self: &Arc<Self>,
        who: Accessor,
        id: &FileId,
        bot: &str,
        langs: &Languages,
    ) -> Result<(), Failure> {
        let (id, bot, langs) = (id.clone(), bot.to_owned(), langs.clone());
        self.blocking(move |files| {
            files.authorize(&id, &who, &langs)?;
            files
                .registry
                .revoke_bot(&id, &bot)?
                .ok_or_else(|| Failure::unknown(&langs))?;
            Ok(())
        })
        .await
    }

    /// 把链接发到“Wonton”私聊：主人所有设备都看得到卡片。发之前先清掉过期（和到期清理排同一个队），发不出去时文件多留一阵也好过链接被删掉。
    pub async fn send_to_devices_for(
        self: &Arc<Self>,
        who: Accessor,
        id: &FileId,
        langs: &Languages,
    ) -> Result<(), Failure> {
        let (task, langs) = (id.clone(), langs.clone());
        let notify = who.clone();
        let turn = self.uploads.turn(id).await;
        let granted = self
            .blocking(move |files| {
                files.authorize(&task, &who, &langs)?;
                files
                    .registry
                    .grant(&task, who, now_secs())?
                    .ok_or_else(|| Failure::unknown(&langs))?;
                Ok(())
            })
            .await;
        drop(turn);
        granted?;
        self.host.notify_person(&notify, &id.link()).await
    }

    /// 删掉编号。设备上传的文件核对后连文件一起删；已经换成别的文件时编号照删、文件不动，返回 `file_changed`。其他来源的
    /// 原文件不动。删除和审计在单独的任务里一起做：请求断了也要做完，文件删了就要有审计。
    pub async fn delete_for(
        self: &Arc<Self>,
        who: Accessor,
        id: &FileId,
        device: String,
        langs: &Languages,
    ) -> Result<(), Failure> {
        let files = self.clone();
        let id = id.clone();
        let langs = langs.clone();
        tokio::spawn(async move {
            // 和放文件排同一个队：不然放到一半的文件可能删完之后才冒出来。
            let _turn = files.uploads.turn(&id).await;
            let task = id.clone();
            let langs = langs.clone();
            let removed = files
                .blocking(move |files| files.delete_now(&task, &who, &langs))
                .await;
            let outcome = match &removed {
                Ok(outcome) => outcome,
                Err(e) => e.kind.as_str(),
            };
            files.audit(&FileEvent {
                event: "file_delete",
                device,
                file: id.as_str(),
                range: None,
                bots: None,
                outcome,
            });
            removed.map(|_| ())
        })
        .await
        .map_err(|e| Failure::internal(format!("删除文件的任务中止：{e}")))?
    }

    /// 返回审计要写的结果。目录没能确认落盘时记录也照删：文件这时已经没了，留着记录主人就删不掉这个编号，断电后文件回来的话审计里有据可查。
    pub(super) fn delete_now(
        &self,
        id: &FileId,
        who: &Accessor,
        langs: &Languages,
    ) -> Result<&'static str, Failure> {
        let record = self.authorize(id, who, langs)?;
        // 删失败了记录留着：文件还在，主人可以再删一次。
        let removed = match &record.origin {
            Origin::Upload {
                reference, placed, ..
            } => {
                // 放好之后清理没做完（崩溃了）的残留先清掉：待完成记录和数据还在的话，下次清理会把删掉的文件再放回来。
                if !self.forget_now(id, true) {
                    return Err(Failure::new(
                        FileFailure::Internal,
                        l10n::text(langs, "files-residual-undeletable", &[]),
                    ));
                }
                self.origin_shares(&record.origin)?
                    .delete_if_unchanged(reference, placed, langs)
                    .map_err(|e| share_failure(e, langs))?
            }
            _ => Deleted::AlreadyGone,
        };
        self.registry.remove(id)?;
        match removed {
            Deleted::Removed { durable: false } => Ok(DELETED_UNSYNCED),
            Deleted::Removed { durable: true } | Deleted::AlreadyGone => Ok("ok"),
            Deleted::Changed => Err(Failure::new(
                FileFailure::FileChanged,
                l10n::text(langs, "files-path-collision", &[]),
            )),
        }
    }

    /// 传完但链接没发出去的上传到期了：和 App 里删同一条核对，记录跟着删。调用方排着这个上传的队，没有请求头，里面的错误按英文说。
    /// 开始之后才授权出去（链接又发了）的不删；名字上已经换成主人的文件时不再过期，留着记录。返回审计要写的结果。
    pub(super) fn delete_expired_now(
        &self,
        id: &FileId,
        now: u64,
    ) -> Result<&'static str, Failure> {
        let record = match self.registry.get(id, now) {
            Lookup::Expired(record) => record,
            // 已经被删掉或者刚刚授权出去。
            _ => return Ok("kept"),
        };
        let Origin::Upload {
            reference, placed, ..
        } = &record.origin
        else {
            return Ok("kept");
        };
        let langs = Languages::from_header(None);
        if !self.forget_now(id, true) {
            return Err(Failure::internal(l10n::text(
                &langs,
                "files-residual-undeletable",
                &[],
            )));
        }
        match self
            .origin_shares(&record.origin)?
            .delete_if_unchanged(reference, placed, &langs)
        {
            Ok(Deleted::AlreadyGone) => {
                self.registry.remove(id)?;
                Ok("expired")
            }
            Ok(Deleted::Removed { durable }) => {
                // 文件已经没了，记录留着就删不掉这个编号。
                self.registry.remove(id)?;
                Ok(if durable { "expired" } else { DELETED_UNSYNCED })
            }
            Ok(Deleted::Changed) => {
                self.registry.keep(id)?;
                Ok("file_changed")
            }
            Err(e) => Err(share_failure(e, &langs)),
        }
    }
}
