//! 假 Docker 的 exec 启动端点。

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::Response;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{FakeDocker, FakeExec, gone};

/// `POST /exec/<id>/start`：和 Docker 一样升级成原始连接，读完标准输入（客户端关闭写端）后按 Docker 的多路
/// 复用格式写回标准错误，再关闭连接。
pub(super) fn start_exec(fake: FakeDocker, id: &str, mut request: Request) -> Response {
    let Some(exec) = fake.lock().execs.get(id).cloned() else {
        return gone(false);
    };
    let failed = fake.lock().exec_start_error.take();
    if let Some((message, started)) = failed {
        if let Some(entry) = fake.lock().execs.get_mut(id) {
            entry.started = started;
        }
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from(message))
            .unwrap();
    }
    if let Some(entry) = fake.lock().execs.get_mut(id) {
        entry.started = true;
    }
    let upgrade = hyper::upgrade::on(&mut request);
    let id = id.to_owned();
    tokio::spawn(async move {
        let mut io = TokioIo::new(upgrade.await.expect("exec 连接升级"));
        let mut stdin = Vec::new();
        io.read_to_end(&mut stdin)
            .await
            .expect("读 exec 的标准输入");
        let handler = fake.lock().exec_handler.clone();
        let output_handler = fake.lock().exec_output_handler.clone();
        let entry_pid = {
            let resources = fake.lock();
            let entry = resources
                .containers
                .get(&exec.container)
                .is_some_and(|labels| {
                    labels.get("wonton.core.component").map(String::as_str) == Some("entry")
                });
            (entry
                && exec
                    .cmd
                    .last()
                    .is_some_and(|command| command.starts_with("printf 'show info")))
            .then(|| 100 + resources.signals.get(&exec.container).copied().unwrap_or(0))
        };
        let (code, stdout, stderr) = match output_handler {
            Some(handler) => handler(&exec.container, &exec.cmd, &stdin),
            None if entry_pid.is_some() => {
                (0, format!("Pid: {}\n", entry_pid.unwrap()), String::new())
            }
            None => match handler {
                Some(handler) => {
                    let (code, stderr) = handler(&exec.container, &exec.cmd, &stdin);
                    (code, String::new(), stderr)
                }
                None => (0, String::new(), String::new()),
            },
        };
        if let Some(exec) = fake.lock().execs.get_mut(&id) {
            exec.stdin = stdin;
            exec.exit_code = Some(code);
        }
        if !stdout.is_empty() {
            let mut frame = vec![1, 0, 0, 0];
            frame.extend_from_slice(&u32::try_from(stdout.len()).unwrap().to_be_bytes());
            frame.extend_from_slice(stdout.as_bytes());
            io.write_all(&frame).await.expect("写 exec 的标准输出");
        }
        if !stderr.is_empty() {
            let mut frame = vec![2, 0, 0, 0];
            frame.extend_from_slice(&u32::try_from(stderr.len()).unwrap().to_be_bytes());
            frame.extend_from_slice(stderr.as_bytes());
            io.write_all(&frame).await.expect("写 exec 的标准错误");
        }
        io.shutdown().await.ok();
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "tcp")
        .body(Body::empty())
        .unwrap()
}

impl FakeDocker {
    /// 容器里执行的命令由 `handler` 回应。
    pub fn on_exec(
        &self,
        handler: impl Fn(&str, &[String], &[u8]) -> (i64, String) + Send + Sync + 'static,
    ) {
        self.lock().exec_handler = Some(Arc::new(handler));
    }

    /// 容器里执行的命令可分别回应标准输出和标准错误。
    pub fn on_exec_output(
        &self,
        handler: impl Fn(&str, &[String], &[u8]) -> (i64, String, String) + Send + Sync + 'static,
    ) {
        self.lock().exec_output_handler = Some(Arc::new(handler));
    }

    /// 执行过的命令，按顺序。
    pub fn execs(&self) -> Vec<FakeExec> {
        self.lock().execs.values().cloned().collect()
    }

    /// 下一次 exec 启动回这个错误：命令已经创建（对账记了账），但连接断了。`started` 说服务端有没有
    /// 把命令跑起来；起来时核心不知道命令的结局，没起来时 Docker 报“没在跑、也没有退出码”。
    /// 两种“结局未知”都不用等超时就能复现（#643）。
    pub fn fail_next_exec_start(&self, message: &str, started: bool) {
        self.lock().exec_start_error = Some((message.to_owned(), started));
    }

    /// 让 Docker 丢掉所有 exec 记录：容器重启后旧 exec 就查不到了。
    pub fn forget_execs(&self) {
        self.lock().execs.clear();
    }

    /// 把还没跑完的最后一个 exec 记成以 `code` 退出（模拟命令在引擎侧完成、核心没等到结果）。
    pub fn finish_last_exec(&self, code: i64) {
        let mut docker = self.lock();
        if let Some((_, exec)) = docker
            .execs
            .iter_mut()
            .filter(|(_, exec)| exec.exit_code.is_none())
            .max_by_key(|(_, exec)| exec.created)
        {
            exec.exit_code = Some(code);
        }
    }
}
