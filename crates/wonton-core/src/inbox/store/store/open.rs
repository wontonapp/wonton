//! 打开收件箱、测试夹具，以及时钟、锁、版本等基础访问。

use super::*;

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

    pub(super) fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("收件箱锁中毒")
    }

    pub(super) fn bump(&self) {
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

    pub(super) fn visible(&self, item: &InboxItem, viewer: &str) -> bool {
        match &item.recipient {
            InboxRecipient::Person { username } => username == viewer,
            InboxRecipient::Admins => self.admins.is_admin(viewer),
            InboxRecipient::Other => false,
        }
    }

    pub(super) fn view_status(&self, item: &InboxItem, viewer: &str) -> InboxItem {
        let mut item = item.clone();
        if matches!(item.recipient, InboxRecipient::Admins)
            && item.status == InboxStatus::Handled
            && item.answered_by.as_deref() != Some(viewer)
        {
            item.status = InboxStatus::HandledElsewhere;
        }
        item
    }
}
