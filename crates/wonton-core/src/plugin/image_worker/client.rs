//! 核心这边：把图片任务交给常驻助手，限时，限并发。
//!
//! 任务 future 被丢掉时（`files` 的缩略图超时就是这样取消的）向助手发一条取消帧，由助手杀掉
//! 那个子进程；助手自己也有和核心一样宽的名额，兜住取消帧晚到的那一下。

use std::collections::HashMap;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use rustix::process::Signal;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, Semaphore, oneshot};

use super::images::{Icon, IconFormat, Screenshot};
use super::protocol::{self, Reply, Request};
use super::{
    Fields, HELPER_GRACE, HOST_COMMAND, ImageError, Job, Languages, MAX_RUNNING, READY_TIMEOUT,
    TIMEOUT, outcome, owner_language_tag, parse_icon,
};
use crate::plugin::pdf::Extracted;

/// 核心这边：把图片任务交给常驻助手，限时，限并发。
pub struct ImageWorker {
    program: Program,
    timeout: Duration,
    running: Semaphore,
    /// 语言文件和主人的用户名，按它读主人的语言（见契约）：给助手和父进程这边的消息用。
    languages: Option<(PathBuf, String)>,
    /// 活着的助手。没起来、退出或者卡住时是空的，下一个任务前再起。
    helper: Mutex<Option<Arc<Helper>>>,
}

/// 助手进程和它的管道。回答由一条读管道分给等着的任务。
struct Helper {
    pid: u32,
    stdin: Mutex<ChildStdin>,
    /// 取出来才能杀；回收由读管道那头等。
    child: Mutex<Option<Child>>,
    pending: StdMutex<HashMap<u32, oneshot::Sender<Result<Reply, String>>>>,
    next_id: AtomicU32,
    closed: AtomicBool,
}

enum Program {
    Path(PathBuf),
    /// 单元测试里没有 `wonton-core` 可执行文件：在线程池里处理，不限资源。
    #[cfg(test)]
    InProcess,
}

impl ImageWorker {
    /// 用正在运行的 `wonton-core` 自己处理。
    pub fn current() -> std::io::Result<Self> {
        Ok(Self::new(std::env::current_exe()?))
    }

    /// 同 [`Self::current`]，文字按这个语言文件里主人的语言写。
    pub fn current_in(languages: PathBuf, owner: String) -> std::io::Result<Self> {
        let mut worker = Self::current()?;
        worker.set_languages(languages, owner);
        Ok(worker)
    }

    /// 用给定的 `wonton-core` 可执行文件处理。
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self::with_program(Program::Path(program.into()))
    }

    #[cfg(test)]
    pub(crate) fn in_process() -> Self {
        Self::with_program(Program::InProcess)
    }

    fn with_program(program: Program) -> Self {
        Self {
            program,
            timeout: TIMEOUT,
            running: Semaphore::new(MAX_RUNNING),
            languages: None,
            helper: Mutex::new(None),
        }
    }

    /// 先把助手起好。核心在绑隧道之前调用：之后起子进程都在助手里面，核心不再在重绑窗口里 fork。
    pub async fn start(&self) -> Result<(), ImageError> {
        // 测试构建里有 InProcess，非测试构建里只有一个分支，clippy 会觉得 match 多余。
        #[allow(clippy::infallible_destructuring_match)]
        let program = match &self.program {
            Program::Path(program) => program,
            #[cfg(test)]
            Program::InProcess => return Ok(()),
        };
        let langs = self.owner_languages();
        self.helper(program, &langs).await.map(|_| ())
    }

    /// 语言文件和主人的用户名，按它读主人的语言，给子进程和这边的消息选语言。
    pub fn set_languages(&mut self, languages: PathBuf, owner: String) {
        self.languages = Some((languages, owner));
    }

    pub(in crate::plugin) fn owner_languages(&self) -> Languages {
        match &self.languages {
            Some((languages, owner)) => Languages::from_header(
                crate::plugin::store::read_people_languages(languages)
                    .get(owner)
                    .map(String::as_str),
            ),
            None => Languages::default(),
        }
    }

    /// 等一个子进程最多多久（默认 10 秒）。
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn icon(&self, format: IconFormat, data: &[u8]) -> Result<Icon, ImageError> {
        let output = self.run(Job::Icon(format), data).await?;
        let langs = self.owner_languages();
        parse_icon(&output, &langs)
    }

    pub async fn tab_icon(&self, format: IconFormat, data: &[u8]) -> Result<Icon, ImageError> {
        let output = self.run(Job::TabIcon(format), data).await?;
        let langs = self.owner_languages();
        parse_icon(&output, &langs)
    }

    pub async fn screenshot(&self, data: &[u8]) -> Result<Screenshot, ImageError> {
        let output = self.run(Job::Screenshot, data).await?;
        let langs = self.owner_languages();
        let mut fields = Fields(&output);
        let shot = Screenshot {
            jpeg: fields.bytes(&langs)?.to_vec(),
            width: fields.number(&langs)?,
            height: fields.number(&langs)?,
        };
        Ok(shot)
    }

    pub async fn thumbnail(&self, data: &[u8]) -> Result<Vec<u8>, ImageError> {
        let output = self.run(Job::Thumbnail, data).await?;
        let langs = self.owner_languages();
        Ok(Fields(&output).bytes(&langs)?.to_vec())
    }

    pub async fn preview(&self, data: &[u8]) -> Result<Vec<u8>, ImageError> {
        let output = self.run(Job::Preview, data).await?;
        let langs = self.owner_languages();
        Ok(Fields(&output).bytes(&langs)?.to_vec())
    }

    /// PDF 的文字层：文字、读到的页数、总页数。
    pub async fn pdf_text(&self, data: &[u8]) -> Result<Extracted, ImageError> {
        let output = self.run(Job::Pdf, data).await?;
        let langs = self.owner_languages();
        let mut fields = Fields(&output);
        Ok(Extracted {
            text: String::from_utf8_lossy(fields.bytes(&langs)?).into_owned(),
            pages_read: fields.number(&langs)?,
            pages_total: fields.number(&langs)?,
            truncated: fields.bytes(&langs)? == [1],
        })
    }

    async fn run(&self, job: Job, input: &[u8]) -> Result<Vec<u8>, ImageError> {
        let _permit = self.running.acquire().await.expect("信号量不会关闭");
        self.execute(job, input).await
    }

    async fn execute(&self, job: Job, input: &[u8]) -> Result<Vec<u8>, ImageError> {
        let langs = self.owner_languages();
        // 同上：测试构建里才有 InProcess。
        #[allow(clippy::infallible_destructuring_match)]
        let program = match &self.program {
            Program::Path(program) => program,
            #[cfg(test)]
            Program::InProcess => {
                let input = input.to_vec();
                return tokio::task::spawn_blocking(move || job.run(&input, &langs))
                    .await
                    .expect("处理图片的线程没有崩溃")
                    .map_err(ImageError::Refused);
            }
        };
        let (helper, id, answers) = self.enqueue(program, &langs, job, input).await?;
        let mut in_flight = InFlight {
            helper: Arc::clone(&helper),
            id,
            done: false,
        };
        let reply = match tokio::time::timeout(self.timeout + HELPER_GRACE, answers).await {
            Ok(Ok(Ok(reply))) => {
                in_flight.done = true;
                reply
            }
            Ok(Ok(Err(reason))) => {
                in_flight.done = true;
                return Err(helper_exited(&langs, &reason));
            }
            Ok(Err(_)) => {
                in_flight.done = true;
                return Err(helper_exited(&langs, "回答的通道断了"));
            }
            Err(_) => {
                // 助手自己卡住了：杀掉它，子进程跟着退。下一个任务前再起。
                in_flight.done = true;
                self.retire(&helper).await;
                return Err(ImageError::TimedOut(crate::l10n::text(
                    &langs,
                    "plugin-install-img-worker-timeout",
                    &[("seconds", self.timeout.as_secs_f32().into())],
                )));
            }
        };
        outcome(
            ExitStatus::from_raw(i32::from(reply.code) << 8),
            reply.output,
            &String::from_utf8_lossy(&reply.message),
            &langs,
        )
    }

    /// 把任务写进助手的标准输入，返回它的回答。写的时候助手正好退出了就再起一个重写：没写进去的任务
    /// 新助手也没收到，重写不算把活重做一遍。
    async fn enqueue(
        &self,
        program: &Path,
        langs: &Languages,
        job: Job,
        input: &[u8],
    ) -> Result<(Arc<Helper>, u32, oneshot::Receiver<Result<Reply, String>>), ImageError> {
        let lang = owner_language_tag(langs);
        let mut last = String::new();
        for _ in 0..2 {
            let helper = self.helper(program, langs).await?;
            let id = helper.next_id.fetch_add(1, Ordering::Relaxed);
            let (send, answers) = oneshot::channel();
            helper
                .pending
                .lock()
                .expect("回答表没被毒化")
                .insert(id, send);
            // 写之前就退了：读管道已经把这个助手上的等待都收走了，重新来。
            if helper.closed.load(Ordering::SeqCst) {
                helper.pending.lock().expect("回答表没被毒化").remove(&id);
                continue;
            }
            let request = Request {
                id,
                timeout: self.timeout,
                job: job.arg(),
                lang: &lang,
                input,
            };
            let written = {
                let mut stdin = helper.stdin.lock().await;
                protocol::write_request(&mut *stdin, &request).await
            };
            match written {
                Ok(()) => return Ok((helper, id, answers)),
                Err(e) => {
                    last = e.to_string();
                    helper.pending.lock().expect("回答表没被毒化").remove(&id);
                    self.retire(&helper).await;
                }
            }
        }
        Err(helper_exited(langs, &last))
    }

    /// 活着的助手，没有就起一个。起的时候持有锁：同时来的任务只起一个。
    async fn helper(&self, program: &Path, langs: &Languages) -> Result<Arc<Helper>, ImageError> {
        let mut slot = self.helper.lock().await;
        if let Some(open) = &*slot
            && !open.closed.load(Ordering::SeqCst)
        {
            return Ok(Arc::clone(open));
        }
        if let Some(dead) = slot.take() {
            let child = dead.child.lock().await.take();
            if let Some(mut child) = child {
                let _ = child.kill().await;
            }
        }
        let helper = start_helper(program, langs).await?;
        *slot = Some(Arc::clone(&helper));
        Ok(helper)
    }

    /// 让一个助手退役：标记关掉、回收进程，下一个任务前再起。已经被换掉的不动。
    async fn retire(&self, helper: &Arc<Helper>) {
        helper.closed.store(true, Ordering::SeqCst);
        {
            let mut slot = self.helper.lock().await;
            if slot.as_ref().is_some_and(|open| Arc::ptr_eq(open, helper)) {
                slot.take();
            }
        }
        let child = helper.child.lock().await.take();
        if let Some(mut child) = child {
            let _ = child.kill().await;
        }
    }
}

/// 还等着的任务。future 被丢掉时（缩略图超时就是取消）告诉助手，让它杀掉子进程。
struct InFlight {
    helper: Arc<Helper>,
    id: u32,
    done: bool,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let helper = Arc::clone(&self.helper);
        let id = self.id;
        runtime.spawn(async move {
            let mut stdin = helper.stdin.lock().await;
            let _ = protocol::write_cancel(&mut *stdin, id).await;
        });
    }
}

impl Drop for ImageWorker {
    fn drop(&mut self) {
        let Some(helper) = self.helper.get_mut().take() else {
            return;
        };
        helper.closed.store(true, Ordering::SeqCst);
        // 读管道那头可能正拿着子进程回收，拿不到就交给它。
        if let Ok(mut child) = helper.child.try_lock()
            && let Some(mut child) = child.take()
        {
            let _ = child.start_kill();
        }
    }
}

/// 起一个助手，等它把协议版本报上来。报上来之前就退了、或者版本对不上，都算起不来。
async fn start_helper(program: &Path, langs: &Languages) -> Result<Arc<Helper>, ImageError> {
    let spawn_failed = |e: std::io::Error| {
        unavailable(
            langs,
            crate::l10n::text(
                langs,
                "plugin-install-img-worker-spawn-failed",
                &[
                    ("program", program.display().to_string().into()),
                    ("error", e.to_string().into()),
                ],
            ),
        )
    };
    let mut command = Command::new(program);
    command
        .arg(HOST_COMMAND)
        .arg(program)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // 核心没了助手跟着退，它再退出时子进程靠各自的 PDEATHSIG 跟着退。
    unsafe {
        command.pre_exec(|| {
            rustix::process::set_parent_process_death_signal(Some(Signal::KILL))
                .map_err(std::io::Error::from)
        });
    }
    let mut child = command.spawn().map_err(spawn_failed)?;
    let pid = child.id().unwrap_or_default();
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        unreachable!("标准输入输出都设成了管道");
    };
    let mut stdout = tokio::io::BufReader::new(stdout);
    let ready = tokio::time::timeout(READY_TIMEOUT, protocol::read_ready(&mut stdout)).await;
    let version = match ready {
        Ok(Ok(version)) => version,
        Ok(Err(e)) => {
            let _ = child.kill().await;
            return Err(not_ready(langs, e.to_string()));
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(not_ready(
                langs,
                format!("{} 秒内没有就绪", READY_TIMEOUT.as_secs()),
            ));
        }
    };
    if version != protocol::PROTOCOL {
        let _ = child.kill().await;
        return Err(unavailable(
            langs,
            crate::l10n::text(
                langs,
                "plugin-install-img-worker-host-protocol",
                &[("got", version.into()), ("want", protocol::PROTOCOL.into())],
            ),
        ));
    }
    let helper = Arc::new(Helper {
        pid,
        stdin: Mutex::new(stdin),
        child: Mutex::new(Some(child)),
        pending: StdMutex::new(HashMap::new()),
        next_id: AtomicU32::new(1),
        closed: AtomicBool::new(false),
    });
    tokio::spawn(read_answers(Arc::clone(&helper), stdout));
    tokio::spawn(forward_stderr(tokio::io::BufReader::new(stderr)));
    tracing::info!(pid, program = %program.display(), "图片助手已启动");
    Ok(helper)
}

/// 把回答交给等着的任务；助手退出时把所有等着的都叫醒，如实带上原因。
async fn read_answers(helper: Arc<Helper>, mut from: tokio::io::BufReader<ChildStdout>) {
    let reason = loop {
        match protocol::read_reply(&mut from).await {
            Ok(Some(reply)) => {
                let waiting = helper
                    .pending
                    .lock()
                    .expect("回答表没被毒化")
                    .remove(&reply.id);
                if let Some(send) = waiting {
                    let _ = send.send(Ok(reply));
                }
            }
            Ok(None) => break "标准输出到了结尾".to_owned(),
            Err(e) => break e.to_string(),
        }
    };
    helper.closed.store(true, Ordering::SeqCst);
    let waiting: Vec<_> = helper
        .pending
        .lock()
        .expect("回答表没被毒化")
        .drain()
        .map(|(_, send)| send)
        .collect();
    for send in waiting {
        let _ = send.send(Err(reason.clone()));
    }
    tracing::debug!(pid = helper.pid, %reason, "图片助手已退出");
    if let Some(mut child) = helper.child.lock().await.take() {
        let _ = child.wait().await;
    }
}

/// 把助手自己打印的话转进日志。
async fn forward_stderr(mut stderr: tokio::io::BufReader<tokio::process::ChildStderr>) {
    let mut line = String::new();
    while let Ok(n) = stderr.read_line(&mut line).await {
        if n == 0 {
            break;
        }
        tracing::debug!("图片助手：{}", line.trim_end());
        line.clear();
    }
}

fn unavailable(langs: &Languages, error: String) -> ImageError {
    ImageError::Unavailable(crate::l10n::text(
        langs,
        "plugin-install-img-worker-unavailable",
        &[("error", error.into())],
    ))
}

fn not_ready(langs: &Languages, error: String) -> ImageError {
    unavailable(
        langs,
        crate::l10n::text(
            langs,
            "plugin-install-img-worker-host-not-ready",
            &[("error", error.into())],
        ),
    )
}

fn helper_exited(langs: &Languages, reason: &str) -> ImageError {
    unavailable(
        langs,
        crate::l10n::text(
            langs,
            "plugin-install-img-worker-host-exited",
            &[("error", reason.into())],
        ),
    )
}
