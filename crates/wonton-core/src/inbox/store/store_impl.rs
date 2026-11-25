//! 收件箱的存储、生命周期、上限和答案。对外只有 [`Inbox`] 一个入口。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use super::*;
use crate::inbox::MAX_WAITERS_PER_ITEM;
use crate::inbox::page::{self, PageTokens};
use tokio::sync::oneshot;
use wonton_protocol::{InboxAnswer, InboxAnswerInput, InboxItem, InboxPayload, InboxStatus};

impl Inbox {
    /// 打开 `dir` 下已有的条目；不存在的目录建好（0700）。
    pub fn open(
        dir: PathBuf,
        clock: Arc<dyn Clock>,
        admins: Arc<dyn AdminRoster>,
    ) -> Result<Arc<Self>, InboxError> {
        std::fs::create_dir_all(dir.join(ITEMS_DIR)).map_err(io)?;
        std::fs::create_dir_all(dir.join(PAGES_DIR)).map_err(io)?;
        let mut records = HashMap::new();
        let entries = std::fs::read_dir(dir.join(ITEMS_DIR)).map_err(io)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".json"))
            {
                // 上次崩溃留下的半截临时文件：不是条目，清掉，别读回。
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(stored) = serde_json::from_slice::<Stored>(&bytes) else {
                continue;
            };
            if stored.item.status == InboxStatus::Archived {
                continue;
            }
            records.insert(
                stored.item.id.clone(),
                Record {
                    stored,
                    waiters: Vec::new(),
                },
            );
        }
        // 页面快照的残留临时文件：同样不是页面，清掉。
        if let Ok(pages) = std::fs::read_dir(dir.join(PAGES_DIR)) {
            for entry in pages.flatten() {
                let path = entry.path();
                if !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".html"))
                {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        let revision = 1 + records.len() as u64;
        let (revision_tx, _) = watch::channel(revision);
        Ok(Arc::new(Self {
            dir,
            inner: Mutex::new(Inner {
                records,
                revision,
                rate: HashMap::new(),
            }),
            revision: revision_tx,
            clock,
            admins,
            devices: RwLock::new(None),
            tokens: PageTokens::default(),
        }))
    }

    /// 只给测试：打开一个临时目录。
    #[cfg(test)]
    pub fn for_test(dir: &Path) -> Arc<Self> {
        let _ = std::fs::create_dir_all(dir.join("items"));
        let _ = std::fs::create_dir_all(dir.join("pages"));
        Arc::new(Self {
            dir: dir.to_owned(),
            inner: Mutex::new(Inner {
                records: HashMap::new(),
                revision: 1,
                rate: HashMap::new(),
            }),
            revision: watch::Sender::new(1),
            clock: Arc::new(SystemClock),
            admins: Arc::new(TestAdmins),
            devices: RwLock::new(None),
            tokens: PageTokens::default(),
        })
    }

    /// 只给测试：拨时钟。
    #[cfg(test)]
    pub fn set_clock(self: &Arc<Self>, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            dir: self.dir.clone(),
            inner: Mutex::new(Inner {
                records: self.inner.lock().unwrap().records_drain(),
                revision: *self.revision.borrow(),
                rate: HashMap::new(),
            }),
            revision: self.revision.clone(),
            clock,
            admins: self.admins.clone(),
            devices: RwLock::new(self.devices.read().expect("设备表锁中毒").clone()),
            tokens: self.tokens.clone(),
        })
    }

    pub fn attach_devices(&self, devices: Arc<dyn DevicePresence>) {
        *self.devices.write().expect("设备表锁中毒") = Some(devices);
    }

    fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("收件箱锁中毒")
    }

    fn bump(&self) {
        let revision = {
            let mut inner = self.lock();
            inner.revision = inner.revision.saturating_add(1);
            inner.revision
        };
        let _ = self.revision.send(revision);
    }

    /// 版本变化的订阅：HTTP 长轮询挂在它上面。
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    pub fn clock_now(&self) -> u64 {
        self.now()
    }

    /// 条目的当前状态；没有时 `None`。
    pub fn status(&self, id: &str) -> Option<InboxStatus> {
        self.lock()
            .records
            .get(id)
            .map(|record| record.stored.item.status)
    }

    pub fn store_dir(&self) -> &Path {
        &self.dir
    }

    pub fn page_tokens(&self) -> PageTokens {
        self.tokens.clone()
    }

    /// 建一条条目。上限和频率按发送方算，超了就报，不截断。
    #[allow(clippy::too_many_lines)]
    pub fn create(&self, mut input: CreateInput) -> Result<CreateReply, InboxError> {
        let now = self.now();
        let page = input.page.take();
        let mut item = self.draft(input, now)?;
        let mut reply_waiter = None;
        let mut warning = None;
        let id = match item.id.as_str() {
            "" => self.next_id(item.kind),
            _ => item.id.clone(),
        };
        item.id = id;
        validate_item(&item).map_err(InboxError::Invalid)?;
        let sender_key = format!("{}:{}", sender_tag(&item.sender), item.sender.id);
        let mut inner = self.lock();
        if let Some(existing) = dedupe_target(&inner, &sender_key, item.dedupe_key.as_deref()) {
            if let Some(page) = &page {
                page::validate_size(page.html.len())?;
            }
            let record = inner.records.get_mut(&existing).expect("刚找到的条目");
            if let Some(page) = &page {
                record.stored.csp = Some(page.csp.clone());
            }
            record.stored.item.title = item.title;
            record.stored.item.body = item.body;
            record.stored.item.payload = item.payload;
            record.stored.item.level = item.level;
            record.stored.item.expires_at_ms = item.expires_at_ms;
            record.stored.item.blocking = item.blocking;
            record.stored.item.sender = item.sender;
            let reopened = record.stored.item.clone();
            if reopened.blocking {
                if record.waiters.len() >= MAX_WAITERS_PER_ITEM {
                    return Err(InboxError::TooManyPending);
                }
                let (tx, rx) = oneshot::channel();
                record.waiters.push(tx);
                reply_waiter = Some(rx);
            }
            let csp = record.stored.csp.clone();
            drop(inner);
            if let Some(page) = &page {
                self.write_page(&reopened.id, &page.html)?;
            }
            self.write(&reopened, csp.as_deref())?;
            self.bump();
            return Ok(CreateReply {
                item: reopened,
                waiter: reply_waiter,
                warning,
            });
        }
        self.check_limits(&inner, &sender_key, item.kind, now)?;
        if let Some(page) = &page {
            page::validate_size(page.html.len())?;
        }
        if let Some(devices) = self.devices.read().expect("设备表锁中毒").as_ref() {
            let person = person_of(&item.recipient);
            if !devices.has_devices(person.as_deref()) {
                warning = Some("no_device".to_owned());
            }
        }
        if item.blocking {
            let (tx, rx) = oneshot::channel();
            reply_waiter = Some(rx);
            inner.records.insert(
                item.id.clone(),
                Record {
                    stored: Stored {
                        item: item.clone(),
                        csp: page.as_ref().map(|p| p.csp.clone()),
                        handled_at_ms: None,
                        terminal_at_ms: None,
                        delivery: None,
                    },
                    waiters: vec![tx],
                },
            );
        } else {
            inner.records.insert(
                item.id.clone(),
                Record {
                    stored: Stored {
                        item: item.clone(),
                        csp: page.as_ref().map(|p| p.csp.clone()),
                        handled_at_ms: None,
                        terminal_at_ms: None,
                        delivery: None,
                    },
                    waiters: Vec::new(),
                },
            );
        }
        inner.revision = inner.revision.saturating_add(1);
        inner.rate.entry(sender_key).or_default().push_back(now);
        drop(inner);
        let csp = page.as_ref().map(|p| p.csp.as_str());
        self.write(&item, csp)?;
        if let Some(page) = page {
            self.write_page(&item.id, &page.html)?;
        }
        self.bump();
        Ok(CreateReply {
            item,
            waiter: reply_waiter,
            warning,
        })
    }

    fn draft(&self, input: CreateInput, now: u64) -> Result<InboxItem, InboxError> {
        let expires = input
            .expires_in_ms
            .unwrap_or_else(|| default_expiry(input.kind, input.blocking));
        let expires = expires.clamp(1, max_expiry(input.kind));
        let title = if input.title.trim().is_empty() {
            fallback_title(&input.payload)
        } else {
            input.title
        };
        Ok(InboxItem {
            id: String::new(),
            kind: input.kind,
            status: InboxStatus::Pending,
            created_at_ms: now,
            expires_at_ms: Some(now.saturating_add(expires)),
            sender: input.sender,
            recipient: input.recipient,
            title,
            body: input.body,
            source: input.source,
            level: input.level,
            blocking: input.blocking,
            dedupe_key: input.dedupe_key,
            progress: None,
            snooze_until_ms: None,
            read_by: Vec::new(),
            answered_by: None,
            answered_at_ms: None,
            outcome: None,
            result: None,
            payload: input.payload,
            raw_payload: None,
            answer: None,
            superseded_answers: Vec::new(),
        })
    }

    fn check_limits(
        &self,
        inner: &Inner,
        sender_key: &str,
        kind: InboxType,
        now: u64,
    ) -> Result<(), InboxError> {
        if let Some(page) = pending_class(kind) {
            let (items, pages) = pending_counts(inner, sender_key);
            if page && pages >= MAX_PENDING_INBOX_PAGES {
                return Err(InboxError::TooManyPending);
            }
            if !page && items >= MAX_PENDING_INBOX_ITEMS {
                return Err(InboxError::TooManyPending);
            }
        }
        let mut rate = inner.rate.get(sender_key).cloned().unwrap_or_default();
        rate.retain(|at| now.saturating_sub(*at) <= DAY_MS);
        let minute = rate
            .iter()
            .filter(|at| now.saturating_sub(**at) <= MINUTE_MS)
            .count();
        if minute >= MAX_INBOX_PER_MINUTE || rate.len() >= MAX_INBOX_PER_DAY {
            return Err(InboxError::RateLimited);
        }
        Ok(())
    }

    fn record_create_rate(&self, sender_key: &str, now: u64) {
        let mut inner = self.lock();
        inner
            .rate
            .entry(sender_key.to_owned())
            .or_default()
            .push_back(now);
    }

    fn next_id(&self, kind: InboxType) -> String {
        let prefix = kind.prefix().unwrap_or('x');
        format!("{prefix}-{}", random_token())
    }

    fn write(&self, item: &InboxItem, csp: Option<&str>) -> Result<(), InboxError> {
        let delivery = self
            .lock()
            .records
            .get(&item.id)
            .and_then(|record| record.stored.delivery);
        let stored = Stored {
            item: item.clone(),
            csp: csp.map(str::to_owned),
            handled_at_ms: item.answered_at_ms,
            terminal_at_ms: terminal_at(item),
            delivery,
        };
        let bytes = serde_json::to_vec(&stored).map_err(|e| io(e.to_string()))?;
        page::write_atomic(
            &self.dir.join(ITEMS_DIR).join(format!("{}.json", item.id)),
            &bytes,
        )
    }

    fn write_page(&self, id: &str, html: &[u8]) -> Result<(), InboxError> {
        page::write_atomic(&self.dir.join(PAGES_DIR).join(format!("{id}.html")), html)
    }

    /// 页面快照；没有页面时 `None`。
    pub fn page(&self, id: &str) -> Result<Option<(Vec<u8>, String)>, InboxError> {
        let inner = self.lock();
        let Some(record) = inner.records.get(id) else {
            return Err(InboxError::NotFound);
        };
        let Some(csp) = record.stored.csp.as_ref() else {
            return Ok(None);
        };
        let path = self.dir.join(PAGES_DIR).join(format!("{id}.html"));
        let html = std::fs::read(path).map_err(io)?;
        Ok(Some((html, csp.clone())))
    }

    /// 页面字节和 CSP；没有页面是 `None`。
    pub fn page_bytes(&self, id: &str) -> Result<Option<(Vec<u8>, String)>, InboxError> {
        self.page(id)
    }

    /// 收件箱列表：只列这个人的。
    pub fn list(&self, viewer: &str, now: u64) -> InboxList {
        let inner = self.lock();
        let mut items = Vec::new();
        let mut counts = InboxCounts::default();
        for record in inner.records.values() {
            if !self.visible(&record.stored.item, viewer) {
                continue;
            }
            if record
                .stored
                .item
                .snooze_until_ms
                .is_some_and(|until| until > now)
            {
                continue;
            }
            let item = self.view_status(&record.stored.item, viewer);
            let summary = summarize(&item, viewer);
            if item.status == InboxStatus::Pending && summary.needs_answer {
                counts.needing += 1;
            }
            if item.kind == InboxType::Notice
                && item.status == InboxStatus::Pending
                && summary.unread
            {
                counts.unread_notices += 1;
            }
            items.push(summary);
        }
        sort_list(&mut items);
        InboxList {
            items,
            counts,
            revision: inner.revision,
        }
    }

    /// 一条条目的详情。
    pub fn view(&self, viewer: &str, id: &str, now: u64) -> Result<InboxView, InboxError> {
        let inner = self.lock();
        let record = inner.records.get(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        let item = self.view_status(&record.stored.item, viewer);
        let page = if record.stored.csp.is_some() {
            Some(wonton_protocol::InboxPageView {
                url: String::new(),
                network_domains: match &item.payload {
                    InboxPayload::Page(payload) => payload.network_domains.clone(),
                    _ => Vec::new(),
                },
                csp: record.stored.csp.clone().unwrap_or_default(),
            })
        } else {
            None
        };
        let can_answer =
            item.status == InboxStatus::Pending && !matches!(item.payload, InboxPayload::Other(_));
        let can_reedit = item.status == InboxStatus::Handled
            && !item.blocking
            && item
                .answered_at_ms
                .is_some_and(|at| now.saturating_sub(at) <= INBOX_REANSWER_WINDOW_SECS * 1000);
        Ok(InboxView {
            item,
            page,
            can_answer,
            can_reedit,
            read_by_me: Some(viewer.to_owned()),
        })
    }

    /// 作答。幂等：同一个 `answer_id` 返回同一条；异步条目 60 秒内可换答案。
    #[allow(clippy::too_many_lines)]
    pub fn answer(
        &self,
        viewer: &str,
        id: &str,
        input: InboxAnswerInput,
    ) -> Result<AnswerOutcome, InboxError> {
        let now = self.now();
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        let item = &mut record.stored.item;
        if let Some(existing) = &item.answer
            && existing.answer_id == input.answer_id
        {
            return Ok(AnswerOutcome {
                item: item.clone(),
                answer: existing.clone(),
                superseded: false,
            });
        }
        match item.status {
            InboxStatus::Pending => {}
            InboxStatus::Expired => return Err(InboxError::Expired),
            InboxStatus::Withdrawn => return Err(InboxError::Withdrawn),
            InboxStatus::Handled | InboxStatus::HandledElsewhere => {
                if item.blocking {
                    return Err(InboxError::AlreadyAnswered);
                }
                let within = item
                    .answered_at_ms
                    .is_some_and(|at| now.saturating_sub(at) <= INBOX_REANSWER_WINDOW_SECS * 1000);
                if !within {
                    return Err(InboxError::AnswerLocked);
                }
            }
            InboxStatus::Archived | InboxStatus::Other => return Err(InboxError::AlreadyAnswered),
        }
        if item.expires_at_ms.is_some_and(|expires| now >= expires) {
            item.status = InboxStatus::Expired;
            record.stored.terminal_at_ms = Some(now);
            let expired = item.clone();
            let waiters = std::mem::take(&mut record.waiters);
            let csp = record.stored.csp.clone();
            drop(inner);
            self.write(&expired, csp.as_deref())?;
            self.bump();
            for waiter in waiters {
                let _ = waiter.send(InboxAnswer {
                    answer_id: "expired".into(),
                    action: InboxAnswerAction::Acknowledge,
                    answers: Vec::new(),
                    action_id: None,
                    comment: Some("过期".into()),
                    answered_at_ms: now,
                    by: None,
                });
            }
            return Err(InboxError::Expired);
        }
        validate_answer(item, &input).map_err(InboxError::Invalid)?;
        let answer = InboxAnswer {
            answer_id: input.answer_id,
            action: input.action,
            answers: input.answers,
            action_id: input.action_id,
            comment: input.comment,
            answered_at_ms: now,
            by: Some(viewer.to_owned()),
        };
        let mut superseded = false;
        if let Some(old) = item.answer.clone() {
            item.superseded_answers.insert(0, old);
            superseded = true;
        }
        item.answer = Some(answer.clone());
        item.status = InboxStatus::Handled;
        item.outcome = Some(match answer.action {
            InboxAnswerAction::Decline => InboxOutcome::Declined,
            InboxAnswerAction::Acknowledge => InboxOutcome::Acknowledged,
            _ => InboxOutcome::Answered,
        });
        item.answered_by = Some(viewer.to_owned());
        if item.answered_at_ms.is_none() {
            item.answered_at_ms = Some(now);
        }
        record.stored.handled_at_ms.get_or_insert(now);
        let waiters = std::mem::take(&mut record.waiters);
        let stored_item = item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        let sender_key = format!(
            "{}:{}",
            sender_tag(&stored_item.sender),
            stored_item.sender.id
        );
        self.record_create_rate(&sender_key, now);
        self.write(&stored_item, csp.as_deref())?;
        self.bump();
        for waiter in waiters {
            let _ = waiter.send(answer.clone());
        }
        Ok(AnswerOutcome {
            item: stored_item,
            answer,
            superseded,
        })
    }

    /// 撤回答案；只有答过的异步条目、60 秒内、本人。
    pub fn withdraw(
        &self,
        viewer: &str,
        id: &str,
        answer_id: &str,
    ) -> Result<InboxItem, InboxError> {
        let now = self.now();
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        let item = &mut record.stored.item;
        if item.blocking {
            return Err(InboxError::AnswerLocked);
        }
        if item.status != InboxStatus::Handled {
            return Err(InboxError::AlreadyAnswered);
        }
        if item.answered_by.as_deref() != Some(viewer) {
            return Err(InboxError::NotYours);
        }
        if !item
            .answered_at_ms
            .is_some_and(|at| now.saturating_sub(at) <= INBOX_REANSWER_WINDOW_SECS * 1000)
        {
            return Err(InboxError::AnswerLocked);
        }
        if item.answer.as_ref().map(|a| a.answer_id.as_str()) != Some(answer_id) {
            return Err(InboxError::NotFound);
        }
        item.status = InboxStatus::Pending;
        item.answered_at_ms = None;
        item.answered_by = None;
        item.outcome = None;
        record.stored.handled_at_ms = None;
        record.stored.terminal_at_ms = None;
        let stored = item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&stored, csp.as_deref())?;
        self.bump();
        Ok(stored)
    }

    /// 已读。
    pub fn read(&self, viewer: &str, id: &str) -> Result<InboxItem, InboxError> {
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        if !record.stored.item.read_by.iter().any(|who| who == viewer) {
            record.stored.item.read_by.push(viewer.to_owned());
        }
        let item = record.stored.item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        Ok(item)
    }

    /// 发送方撤回。
    pub fn cancel(
        &self,
        viewer: &str,
        id: &str,
        reason: Option<String>,
    ) -> Result<InboxItem, InboxError> {
        let now = self.now();
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        if record.stored.item.status != InboxStatus::Pending {
            return Err(InboxError::AlreadyAnswered);
        }
        record.stored.item.status = InboxStatus::Withdrawn;
        record.stored.item.result = reason;
        record.stored.terminal_at_ms = Some(now);
        let item = record.stored.item.clone();
        let waiters = std::mem::take(&mut record.waiters);
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        for waiter in waiters {
            // 撤回也发一条空答案，阻塞的发送方知道结束了。
            let _ = waiter.send(InboxAnswer {
                answer_id: "withdrawn".into(),
                action: InboxAnswerAction::Acknowledge,
                answers: Vec::new(),
                action_id: None,
                comment: item.result.clone(),
                answered_at_ms: now,
                by: Some(viewer.to_owned()),
            });
        }
        Ok(item)
    }

    /// 稍后提醒：只挪出列表，不延长期限。
    pub fn snooze(
        &self,
        viewer: &str,
        id: &str,
        preset: InboxSnoozePreset,
        timezone_offset_min: Option<i32>,
    ) -> Result<InboxItem, InboxError> {
        let now = self.now();
        let until = snooze_until(preset, now, timezone_offset_min.unwrap_or(0))?;
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        if record.stored.item.status != InboxStatus::Pending {
            return Err(InboxError::AlreadyAnswered);
        }
        record.stored.item.snooze_until_ms = Some(until);
        let item = record.stored.item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        Ok(item)
    }

    fn visible(&self, item: &InboxItem, viewer: &str) -> bool {
        match &item.recipient {
            InboxRecipient::Person { username } => username == viewer,
            InboxRecipient::Admins => self.admins.is_admin(viewer),
            InboxRecipient::Other => false,
        }
    }

    fn view_status(&self, item: &InboxItem, viewer: &str) -> InboxItem {
        let mut item = item.clone();
        if matches!(item.recipient, InboxRecipient::Admins)
            && item.status == InboxStatus::Handled
            && item.answered_by.as_deref() != Some(viewer)
        {
            item.status = InboxStatus::HandledElsewhere;
        }
        item
    }

    /// 把到期的标记成过期、过保留期的归档/删掉。后台每 30 秒跑一次；测试直接调。
    pub fn sweep(&self, now: u64) {
        let mut inner = self.lock();
        let mut changed: Vec<(InboxItem, Option<String>)> = Vec::new();
        inner.records.retain(|id, record| {
            let item = &mut record.stored.item;
            if item.status == InboxStatus::Pending
                && item.expires_at_ms.is_some_and(|expires| now >= expires)
            {
                item.status = InboxStatus::Expired;
                record.stored.terminal_at_ms = Some(now);
                changed.push((item.clone(), record.stored.csp.clone()));
                return true;
            }
            if item.status == InboxStatus::Handled
                && record
                    .stored
                    .handled_at_ms
                    .is_some_and(|at| now.saturating_sub(at) > DAY_MS)
            {
                item.status = InboxStatus::Archived;
                record.stored.terminal_at_ms = Some(now);
                changed.push((item.clone(), record.stored.csp.clone()));
                return true;
            }
            if item.status == InboxStatus::Archived
                && record
                    .stored
                    .terminal_at_ms
                    .is_some_and(|at| now.saturating_sub(at) > 30 * DAY_MS)
            {
                let _ = std::fs::remove_file(self.dir.join(PAGES_DIR).join(format!("{id}.html")));
                let _ = std::fs::remove_file(self.dir.join(ITEMS_DIR).join(format!("{id}.json")));
                return false;
            }
            if matches!(item.status, InboxStatus::Expired | InboxStatus::Withdrawn)
                && record
                    .stored
                    .terminal_at_ms
                    .is_some_and(|at| now.saturating_sub(at) > 30 * DAY_MS)
            {
                let _ = std::fs::remove_file(self.dir.join(PAGES_DIR).join(format!("{id}.html")));
                let _ = std::fs::remove_file(self.dir.join(ITEMS_DIR).join(format!("{id}.json")));
                return false;
            }
            true
        });
        let changed_items = changed;
        drop(inner);
        for (item, csp) in changed_items {
            let _ = self.write(&item, csp.as_deref());
        }
        self.bump();
    }

    /// 阻塞地等某条条目的答案；只有发送方能等，别的 Bot 知道 id 也读不到。
    pub async fn wait_for_sender(
        &self,
        sender_id: &str,
        id: &str,
        after_revision: Option<u64>,
    ) -> Result<InboxItem, InboxError> {
        let mut revision = self.subscribe();
        loop {
            let now = self.now();
            {
                let inner = self.lock();
                let record = inner.records.get(id).ok_or(InboxError::NotFound)?;
                let item = &record.stored.item;
                if item.sender.id != sender_id {
                    return Err(InboxError::NotYours);
                }
                let reached = matches!(
                    item.status,
                    InboxStatus::Handled
                        | InboxStatus::HandledElsewhere
                        | InboxStatus::Expired
                        | InboxStatus::Withdrawn
                        | InboxStatus::Archived
                );
                if reached || after_revision.is_none_or(|after| inner.revision > after) {
                    let _ = now;
                    return Ok(item.clone());
                }
            }
            if revision.changed().await.is_err() {
                return Err(InboxError::ServiceUnavailable);
            }
        }
    }
}

impl Inbox {
    /// 看得到某条条目的发送方本人（不看收件人），撤回/更新用它。
    pub fn visible_to_sender(&self, sender_id: &str, id: &str) -> bool {
        let inner = self.lock();
        inner
            .records
            .get(id)
            .is_some_and(|record| record.stored.item.sender.id == sender_id)
    }

    /// 收件人撤回：按收件人/管理员校验；HTTP 的已读入口用它。
    pub fn cancel_by_viewer(
        &self,
        viewer: &str,
        id: &str,
        reason: Option<String>,
    ) -> Result<InboxItem, InboxError> {
        let inner = self.lock();
        let record = inner.records.get(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        drop(inner);
        self.cancel_raw(id, reason)
    }

    /// 发送方撤回：和 `cancel` 同义，只是按发送方 id 校验。
    pub fn cancel_by_sender(
        &self,
        sender_id: &str,
        id: &str,
        reason: Option<String>,
    ) -> Result<InboxItem, InboxError> {
        if !self.visible_to_sender(sender_id, id) {
            return Err(InboxError::NotYours);
        }
        self.cancel_raw(id, reason)
    }

    fn cancel_raw(&self, id: &str, reason: Option<String>) -> Result<InboxItem, InboxError> {
        let now = self.now();
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if record.stored.item.status != InboxStatus::Pending {
            return Err(InboxError::AlreadyAnswered);
        }
        record.stored.item.status = InboxStatus::Withdrawn;
        record.stored.item.result = reason;
        record.stored.terminal_at_ms = Some(now);
        let item = record.stored.item.clone();
        let waiters = std::mem::take(&mut record.waiters);
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        let answer = InboxAnswer {
            answer_id: "withdrawn".into(),
            action: InboxAnswerAction::Acknowledge,
            answers: Vec::new(),
            action_id: None,
            comment: item.result.clone(),
            answered_at_ms: now,
            by: None,
        };
        for waiter in waiters {
            let _ = waiter.send(answer.clone());
        }
        Ok(item)
    }

    /// 发送方更新还没答的条目：摘要、进度，不新建。
    pub fn update_by_sender(
        &self,
        sender_id: &str,
        id: &str,
        summary: Option<String>,
        progress: Option<f32>,
    ) -> Result<InboxItem, InboxError> {
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if record.stored.item.sender.id != sender_id {
            return Err(InboxError::NotYours);
        }
        if record.stored.item.status != InboxStatus::Pending {
            return Err(InboxError::AlreadyAnswered);
        }
        if let Some(summary) = summary {
            record.stored.item.body = summary;
        }
        if let Some(progress) = progress {
            if !(0.0..=1.0).contains(&progress) {
                return Err(InboxError::Invalid(InboxInvalid::Invalid {
                    field: "progress".into(),
                    value: progress.to_string(),
                    reason: "必须在 0 和 1 之间".into(),
                }));
            }
            record.stored.item.progress = Some(progress);
        }
        let item = record.stored.item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        Ok(item)
    }

    /// 某个 worker 最近一条条目。
    pub fn latest_for_worker(
        &self,
        worker_id: &str,
    ) -> Option<(InboxItem, Option<wonton_protocol::InboxDelivery>)> {
        let inner = self.lock();
        inner
            .records
            .values()
            .filter(|record| {
                record
                    .stored
                    .item
                    .source
                    .as_ref()
                    .and_then(|source| source.worker_id.as_deref())
                    == Some(worker_id)
            })
            .max_by_key(|record| record.stored.item.created_at_ms)
            .map(|record| (record.stored.item.clone(), record.stored.delivery))
    }

    /// worker 投递结果：写到条目结果上。
    pub fn resolve_worker(
        &self,
        worker_id: &str,
        delivery: wonton_protocol::InboxDelivery,
        error: Option<String>,
    ) -> Result<InboxItem, InboxError> {
        let mut inner = self.lock();
        let id = inner
            .records
            .values()
            .filter(|record| {
                record
                    .stored
                    .item
                    .source
                    .as_ref()
                    .and_then(|source| source.worker_id.as_deref())
                    == Some(worker_id)
            })
            .max_by_key(|record| record.stored.item.created_at_ms)
            .map(|record| record.stored.item.id.clone())
            .ok_or(InboxError::NotFound)?;
        let record = inner.records.get_mut(&id).expect("刚找到的条目");
        record.stored.delivery = Some(delivery);
        record.stored.item.result = Some(match error {
            Some(error) => format!("{delivery:?}: {error}"),
            None => format!("{delivery:?}"),
        });
        let item = record.stored.item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        Ok(item)
    }
}
