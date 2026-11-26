//! 条目的建立、读写、列表、详情和 worker 投递。

use super::*;

impl Inbox {
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

    fn next_id(&self, kind: InboxType) -> String {
        let prefix = kind.prefix().unwrap_or('x');
        format!("{prefix}-{}", random_token())
    }

    pub(super) fn write(&self, item: &InboxItem, csp: Option<&str>) -> Result<(), InboxError> {
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
