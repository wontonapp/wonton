//! 文件服务测试的替身：记下发给主人的消息、按预设回答的核心。只存在于测试里。

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use wonton_protocol::{FileFailure, FileId, FileSource};

use super::record::{Accessor, FileRecord, Origin};
use super::resource::InternalUrl;
use super::share_fs::{Names, ShareRef, Shares, TRANSFERS};
use super::{Failure, FileHost, Files, FilesConfig};
use crate::http::HttpClient;
use crate::mcp::audit::Audit;
use crate::plugin::PluginId;
use crate::plugin::hostdir::Owner;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;

/// 测试不挑语言：空的 `Languages` 协商成英文。
pub(crate) fn any_langs() -> Languages {
    Languages::from_header(None)
}

/// 预设回答的核心。
#[derive(Default)]
pub struct FakeHost {
    /// 在运行的插件：服务名到 `host:port`。
    pub services: Mutex<BTreeMap<String, BTreeMap<String, String>>>,
    /// `resources/read` 的结果。
    pub resources: Mutex<BTreeMap<String, Vec<u8>>>,
    /// 读过几次 `resources/read`。
    pub resource_reads: std::sync::atomic::AtomicUsize,
    /// 房间到 Bot。
    pub rooms: Mutex<BTreeMap<String, Vec<String>>>,
    /// 发给主人的消息。
    pub notices: Mutex<Vec<String>>,
    /// 发不出去。
    pub notices_broken: std::sync::atomic::AtomicBool,
    /// 在运行但不提供 MCP 服务的插件。
    pub without_mcp: Mutex<Vec<String>>,
    /// `resources/read` 一直不回答。
    pub reads_hang: std::sync::atomic::AtomicBool,
    /// 只有这个插件的 `resources/read` 一直不回答。
    pub reads_hang_for: Mutex<Option<String>>,
    /// `resources/read` 超过上限。
    pub reads_too_large: std::sync::atomic::AtomicBool,
    /// 所有插件的授权都收回了的共享文件夹。
    pub revoked_shares: Mutex<Vec<String>>,
    /// 这个插件的授权收回了的共享文件夹。
    pub revoked_for: Mutex<Vec<(String, String)>>,
    /// 正在做的 `resources/read`。
    pub reads_running: std::sync::atomic::AtomicUsize,
}

/// 读取结束或者被丢掉时减一。
struct RunningRead<'a>(&'a std::sync::atomic::AtomicUsize);

impl<'a> RunningRead<'a> {
    fn start(count: &'a std::sync::atomic::AtomicUsize) -> Self {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for RunningRead<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl FileHost for FakeHost {
    fn check_plugin(&self, plugin: &PluginId) -> Result<(), Failure> {
        if self.services.lock().unwrap().contains_key(plugin.as_str()) {
            Ok(())
        } else {
            Err(Failure::new(FileFailure::PluginUnavailable, "插件没在运行"))
        }
    }

    fn check_mcp(&self, plugin: &PluginId) -> Result<(), Failure> {
        self.check_plugin(plugin)?;
        if self
            .without_mcp
            .lock()
            .unwrap()
            .iter()
            .any(|p| p == plugin.as_str())
        {
            return Err(Failure::new(
                FileFailure::PluginUnavailable,
                "没有 MCP 服务",
            ));
        }
        Ok(())
    }

    fn service_upstream<'a>(
        &'a self,
        plugin: &'a PluginId,
        url: &'a InternalUrl,
    ) -> BoxFuture<'a, Result<String, Failure>> {
        async move {
            self.check_plugin(plugin)?;
            let services = self.services.lock().unwrap();
            services[plugin.as_str()]
                .get(&url.service)
                .cloned()
                .ok_or_else(|| Failure::new(FileFailure::PermissionDenied, "不是它自己的服务"))
        }
        .boxed()
    }

    fn read_resource<'a>(
        &'a self,
        plugin: &'a PluginId,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Failure>> {
        async move {
            self.check_mcp(plugin)?;
            let _running = RunningRead::start(&self.reads_running);
            let stuck = self.reads_hang_for.lock().unwrap().as_deref() == Some(plugin.as_str());
            if stuck || self.reads_hang.load(std::sync::atomic::Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self
                .reads_too_large
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(Failure::new(FileFailure::ResourceTooLarge, "超过上限"));
            }
            let resources = self.resources.lock().unwrap();
            self.resource_reads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            resources
                .get(uri)
                .cloned()
                .ok_or_else(|| Failure::new(FileFailure::UpstreamError, "插件读不了这个资源"))
        }
        .boxed()
    }

    fn share_granted(&self, plugin: &PluginId, share: &str) -> Result<bool, Failure> {
        let revoked = self.revoked_shares.lock().unwrap();
        let revoked_for = self.revoked_for.lock().unwrap();
        Ok(!revoked.iter().any(|s| s == share)
            && !revoked_for
                .iter()
                .any(|(p, s)| p == plugin.as_str() && s == share))
    }

    fn bots_in_room(&self, room: &str) -> Result<Vec<String>, Failure> {
        let rooms = self.rooms.lock().unwrap();
        Ok(rooms.get(room).cloned().unwrap_or_default())
    }

    fn notify_owner<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        async move {
            if self
                .notices_broken
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(Failure::new(FileFailure::ServiceUnavailable, "发不到私聊"));
            }
            self.notices.lock().unwrap().push(text.to_owned());
            Ok(())
        }
        .boxed()
    }
}

/// 一个临时目录里的文件服务。
pub struct Fixture {
    pub files: Arc<Files>,
    pub host: Arc<FakeHost>,
    pub shares_dir: PathBuf,
    pub audit_path: PathBuf,
    pub owner: Owner,
    pub dir: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let meta = fs::metadata(dir.path()).unwrap();
        let owner = Owner {
            uid: meta.uid(),
            gid: meta.gid(),
        };
        let shares_dir = dir.path().join("shares");
        let audit_path = dir.path().join("audit.jsonl");
        let host = Arc::new(FakeHost::default());
        let config = FilesConfig {
            dir: dir.path().join("files"),
            shares_dir: shares_dir.clone(),
            owner,
        };
        let files = Files::open(
            &config,
            host.clone(),
            HttpClient::new(),
            Arc::new(Audit::new(audit_path.clone())),
            Arc::new(ImageWorker::in_process()),
        )
        .unwrap();
        Self {
            files: Arc::new(files),
            host,
            shares_dir,
            audit_path,
            owner,
            dir,
        }
    }

    /// 重启核心：同样的目录再打开一次。
    pub fn reopen(&self) -> Arc<Files> {
        let config = FilesConfig {
            dir: self.dir.path().join("files"),
            shares_dir: self.shares_dir.clone(),
            owner: self.owner,
        };
        let files = Files::open(
            &config,
            self.host.clone(),
            HttpClient::new(),
            Arc::new(Audit::new(self.audit_path.clone())),
            Arc::new(ImageWorker::in_process()),
        )
        .unwrap();
        Arc::new(files)
    }

    /// 在共享文件夹里放一个文件。
    pub fn put(&self, path: &str, content: &[u8]) -> PathBuf {
        let full = self.shares_dir.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, content).unwrap();
        full
    }

    /// 主人能看的一条共享文件夹记录。
    pub fn shared_with_owner(&self, path: &str, content: &[u8]) -> FileId {
        self.put(path, content);
        let reference = ShareRef::parse(&format!("wonton-share:{path}")).unwrap();
        let name = reference.file_name().to_owned();
        self.save(&name, Origin::Share { reference }, [Accessor::Owner].into())
    }

    /// 只有 `bot` 能看的一条记录。
    pub fn shared_with_bot(&self, path: &str, bot: &str) -> FileId {
        self.put(path, b"bot");
        let reference = ShareRef::parse(&format!("wonton-share:{path}")).unwrap();
        let name = reference.file_name().to_owned();
        let access = [Accessor::Bot(bot.to_owned())].into();
        self.save(&name, Origin::Share { reference }, access)
    }

    /// 设备上传的一条记录：文件已经在 `transfers` 里。
    pub fn uploaded(&self, name: &str, content: &[u8]) -> FileId {
        let incoming = self.dir.path().join("incoming");
        fs::create_dir_all(&incoming).unwrap();
        let data = super::new_id();
        fs::write(incoming.join(data.as_str()), content).unwrap();
        let shares = Shares::new(self.shares_dir.clone());
        let (name, placed) = shares
            .place_upload(
                &incoming,
                data.as_str(),
                content.len() as u64,
                Names {
                    preferred: name,
                    owner: self.owner,
                    placing_inode: None,
                },
                &Languages::from_header(None),
                &mut |_, _| Ok(()),
            )
            .unwrap();
        fs::remove_file(incoming.join(data.as_str())).unwrap();
        shares.remove_upload_temp(data.as_str()).unwrap();
        let reference = ShareRef {
            share: TRANSFERS.to_owned(),
            path: vec![name.clone()],
        };
        self.save(
            &name,
            Origin::Upload { reference, placed },
            [Accessor::Owner].into(),
        )
    }

    pub fn save(
        &self,
        name: &str,
        origin: Origin,
        access: std::collections::BTreeSet<Accessor>,
    ) -> FileId {
        let id = super::new_id();
        self.files
            .registry
            .save(FileRecord {
                id: id.clone(),
                name: name.to_owned(),
                mime_type: None,
                declared_size: None,
                source: FileSource::Device,
                origin,
                access,
                owner_granted: Default::default(),
                registered: None,
                created_at: now_secs(),
                expires_at: None,
            })
            .unwrap();
        id
    }

    /// 记录里写上插件声明的大小。
    pub fn declare_size(&self, id: &FileId, size: u64) {
        let super::record::Lookup::Found(mut record) = self.files.registry.get(id, now_secs())
        else {
            panic!("没有 {id}");
        };
        record.declared_size = Some(size);
        self.files.registry.save(record).unwrap();
    }

    /// tusd 存字节的目录。
    pub fn uploads_dir(&self) -> PathBuf {
        self.dir.path().join("files/uploads")
    }

    /// 审计日志的每一行。
    pub fn audit_lines(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(&self.audit_path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// `wonton-share:` 引用，测试里拼路径用。
pub(crate) fn share(path: &str) -> ShareRef {
    ShareRef::parse(&format!("wonton-share:{path}")).unwrap()
}
