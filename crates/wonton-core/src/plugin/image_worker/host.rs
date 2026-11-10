//! 常驻的图片助手：核心启动时、隧道绑 UDP 之前就起好（`wonton-core image-host <程序>`），每个图片任务
//! 由它再起一个限了内存和时间的子进程。
//!
//! 为什么要多这一层：fork 到 exec 之间，子进程手里有父进程全部 fd 的副本（CLOEXEC 要到 exec 才生效），
//! 包括隧道的 UDP 套接字。iroh 判定网络大变时在原端口重绑，正好撞上这个窗口就 `AddrInUse`；netwatch
//! 0.19.3 重绑失败后停在 `Closed` 而 `is_broken` 还是 false，之后不会再试。核心只在启动时 fork 一次
//! （这时隧道还没绑），图片任务只在助手进程里 fork，重绑就撞不上。助手退出后，核心如实报“图片处理
//! 不可用”，并在下一个任务前再起一个——那一次 fork 又发生在核心，会和重绑撞窗口，只限重起的那一下。
//!
//! 助手不是沙箱，和核心同一个用户：它只负责起限了资源、可被杀掉的子进程。核心把任务 future 丢掉
//! 时发取消帧，助手杀掉对应的子进程；助手自己也限同样的并发，取消帧晚到也不会堆起来。助手和子进程
//! 都设了 `PR_SET_PDEATHSIG`，核心没了它们跟着退出。

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex as StdMutex};

use rustix::process::Signal;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{Mutex, Semaphore, oneshot};

use super::protocol::{self, Incoming, Reply};
use super::{
    BROKEN, COMMAND, ImageError, Job, MAX_OUTPUT, MAX_RUNNING, MAX_STDERR, outcome, read_capped,
};
use crate::plugin::languages::Languages;

/// 助手入口。退出码和 [`super::child`] 一样：0 正常（核心那头结束），2 自己这边出问题。
pub fn host(args: &[String]) -> ExitCode {
    let Some(program) = args.first() else {
        eprintln!("用法：wonton-core {} <程序>", protocol::HOST_COMMAND);
        return ExitCode::from(BROKEN);
    };
    let program = PathBuf::from(program);
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("起不了图片助手的运行时：{e}");
            return ExitCode::from(BROKEN);
        }
    };
    match runtime.block_on(serve(&program)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("图片助手退出：{e}");
            ExitCode::from(BROKEN)
        }
    }
}

/// 读任务、起子进程；核心关掉标准输入或者读写失败就结束。
async fn serve(program: &Path) -> io::Result<()> {
    let mut requests = tokio::io::BufReader::new(tokio::io::stdin());
    let answers = Arc::new(Mutex::new(tokio::io::stdout()));
    let slots = Arc::new(Semaphore::new(MAX_RUNNING));
    // 还等着的任务，取消帧按任务号找它。核心把任务 future 丢了（缩略图超时）就会发取消。
    let cancels: Arc<StdMutex<HashMap<u32, oneshot::Sender<()>>>> =
        Arc::new(StdMutex::new(HashMap::new()));
    protocol::write_ready(&mut *answers.lock().await).await?;
    while let Some(request) = protocol::read_request(&mut requests).await? {
        if request.job == "cancel" {
            let waiting = cancels.lock().expect("取消表没被毒化").remove(&request.id);
            if let Some(cancel) = waiting {
                let _ = cancel.send(());
            }
            continue;
        }
        let id = request.id;
        let (cancel, cancelled) = oneshot::channel();
        cancels.lock().expect("取消表没被毒化").insert(id, cancel);
        let program = program.to_owned();
        let answers = Arc::clone(&answers);
        let slots = Arc::clone(&slots);
        let cancels = Arc::clone(&cancels);
        tokio::spawn(async move {
            // 和核心一样宽的名额：取消帧晚到时，子进程也不会越过上限堆起来。
            let slot = slots.acquire_owned().await.expect("名额不会关闭");
            let reply = tokio::select! {
                _ = cancelled => Reply::refused(id, String::new()),
                reply = handle(&program, request) => reply,
            };
            drop(slot);
            cancels.lock().expect("取消表没被毒化").remove(&id);
            if let Err(e) = protocol::write_reply(&mut *answers.lock().await, &reply).await {
                tracing::warn!("写图片助手的回答失败：{e}");
            }
        });
    }
    Ok(())
}

async fn handle(program: &Path, request: Incoming) -> Reply {
    let langs = Languages::from_header(Some(&request.lang));
    let Some(job) = Job::parse(&request.job) else {
        return Reply::broken(
            request.id,
            crate::l10n::text(
                &langs,
                "plugin-install-img-worker-usage",
                &[("command", COMMAND.into())],
            ),
        );
    };
    match run(program, job, &request).await {
        Ok(output) => Reply::ok(request.id, output),
        Err(ImageError::Refused(reason)) => Reply::refused(request.id, reason),
        Err(ImageError::Unavailable(reason)) => Reply::broken(request.id, reason),
    }
}

/// 在限了内存和时间的子进程里处理一张图。到点由助手杀掉它；助手自己被核心杀掉时，子进程靠 `PR_SET_PDEATHSIG` 跟着退。
async fn run(program: &Path, job: Job, request: &Incoming) -> Result<Vec<u8>, ImageError> {
    let langs = Languages::from_header(Some(&request.lang));
    let mut command = Command::new(program);
    command
        .args([COMMAND, job.arg()])
        .env_clear()
        .env("WONTON_LANG", &request.lang)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // fork 出来的子进程在 core 或助手被 SIGKILL 时跟着死，不会留下来占着管道和内存。
    unsafe {
        command.pre_exec(|| {
            rustix::process::set_parent_process_death_signal(Some(Signal::KILL))
                .map_err(io::Error::from)
        });
    }
    let mut child = command.spawn().map_err(|e| {
        unavailable(
            &langs,
            crate::l10n::text(
                &langs,
                "plugin-install-img-worker-spawn-failed",
                &[
                    ("program", program.display().to_string().into()),
                    ("error", e.to_string().into()),
                ],
            ),
        )
    })?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        unreachable!("标准输入输出都设成了管道");
    };
    let (mut output, mut message) = (Vec::new(), Vec::new());
    let finished = tokio::time::timeout(request.timeout, async {
        let write = async {
            // 子进程读到自己要的上限就不读了，写不进去也照常看退出状态。
            let _ = stdin.write_all(&request.input).await;
            drop(stdin);
        };
        let (_, out, err) = tokio::join!(
            write,
            read_capped(stdout, MAX_OUTPUT, &mut output),
            read_capped(stderr, MAX_STDERR, &mut message),
        );
        out.and(err)?;
        child.wait().await
    })
    .await;
    let status = match finished {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            return Err(unavailable(
                &langs,
                crate::l10n::text(
                    &langs,
                    "plugin-install-img-worker-communicate",
                    &[("error", e.to_string().into())],
                ),
            ));
        }
        Err(_) => {
            // kill 会等进程退出并回收，返回时子进程已经不在了。
            if let Err(e) = child.kill().await {
                tracing::warn!("结束处理图片的子进程失败：{e}");
            }
            return Err(ImageError::Refused(crate::l10n::text(
                &langs,
                "plugin-install-img-worker-timeout",
                &[("seconds", request.timeout.as_secs_f32().into())],
            )));
        }
    };
    outcome(status, output, &String::from_utf8_lossy(&message), &langs)
}

fn unavailable(langs: &Languages, error: String) -> ImageError {
    ImageError::Unavailable(crate::l10n::text(
        langs,
        "plugin-install-img-worker-unavailable",
        &[("error", error.into())],
    ))
}
