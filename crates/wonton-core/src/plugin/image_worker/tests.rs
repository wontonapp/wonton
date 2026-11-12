//! 子进程的退出状态怎么算。真的起子进程的测试在 `tests/image_worker.rs`（要 `wonton-core` 可执行文件）。

use super::*;

/// 测试里没有主人的语言文件：按目录的默认语言断言。
fn langs() -> Languages {
    Languages::default()
}

/// wait(2) 的状态编码：被信号结束时低 7 位是信号，正常退出时退出码在第 8 到 15 位。
fn exited(code: i32) -> ExitStatus {
    ExitStatus::from_raw(code << 8)
}

fn signaled(signal: Signal) -> ExitStatus {
    ExitStatus::from_raw(signal.as_raw())
}

fn refused(status: ExitStatus, stderr: &str) -> String {
    match outcome(status, Vec::new(), stderr, &langs()) {
        Err(ImageError::Refused(reason)) => reason,
        other => panic!("{other:?}"),
    }
}

fn timed_out(status: ExitStatus, stderr: &str) -> String {
    match outcome(status, Vec::new(), stderr, &langs()) {
        Err(ImageError::TimedOut(reason)) => reason,
        other => panic!("{other:?}"),
    }
}

#[test]
fn exit_statuses_tell_the_image_apart_from_the_worker() {
    assert_eq!(
        outcome(exited(0), b"out".to_vec(), "", &langs()),
        Ok(b"out".to_vec())
    );
    let langs = langs();
    assert_eq!(
        refused(
            exited(1),
            &crate::l10n::text(
                &langs,
                "plugin-install-img-not-square",
                &[("width", 48u32.into()), ("height", 24u32.into())],
            ),
        ),
        crate::l10n::text(
            &langs,
            "plugin-install-img-not-square",
            &[("width", 48u32.into()), ("height", 24u32.into())],
        )
    );
    // 起不来、设不了上限：不是图片的问题，不该记成这张图处理不了。
    let rlimit = crate::l10n::text(
        &langs,
        "plugin-install-img-worker-rlimit",
        &[("error", "EPERM".into())],
    );
    assert_eq!(
        outcome(exited(2), Vec::new(), &rlimit, &langs),
        Err(ImageError::Unavailable(rlimit))
    );
}

#[test]
fn crashes_and_limits_are_reported_as_unprocessable_with_the_cause() {
    let langs = langs();
    assert!(
        refused(exited(101), "thread 'main' panicked at …").contains(&crate::l10n::text(
            &langs,
            "plugin-install-img-worker-crashed",
            &[("code", 101i32.into())]
        ))
    );
    let oom = refused(
        signaled(Signal::ABORT),
        "memory allocation of 1073741824 bytes failed",
    );
    assert!(
        oom.contains(&crate::l10n::text(
            &langs,
            "plugin-install-img-worker-memory-limit",
            &[("mib", (MEMORY_LIMIT >> 20).into())],
        )),
        "{oom}"
    );
    assert!(
        refused(signaled(Signal::KILL), "").contains(&crate::l10n::text(
            &langs,
            "plugin-install-img-worker-killed",
            &[]
        ))
    );
    let term = Signal::TERM.as_raw();
    assert!(
        refused(signaled(Signal::TERM), "").contains(&crate::l10n::text(
            &langs,
            "plugin-install-img-worker-signal",
            &[("signal", term.into())]
        ))
    );
}

#[test]
fn a_cpu_timeout_is_not_the_files_fault() {
    let langs = langs();
    assert!(
        timed_out(signaled(Signal::XCPU), "").contains(&crate::l10n::text(
            &langs,
            "plugin-install-img-worker-cpu-timeout",
            &[("seconds", CPU_SECONDS.into())],
        ))
    );
    // 助手说超时的结果码也一样。
    assert_eq!(
        outcome(
            ExitStatus::from_raw(i32::from(TIMED_OUT) << 8),
            Vec::new(),
            "太慢",
            &langs
        ),
        Err(ImageError::TimedOut("太慢".to_owned()))
    );
}

#[test]
fn malformed_output_is_the_workers_fault() {
    let langs = langs();
    let mut out = Vec::new();
    put(&mut out, b"jpeg");
    put(&mut out, &7u32.to_le_bytes());
    let mut fields = Fields(&out);
    assert_eq!(fields.bytes(&langs), Ok(&b"jpeg"[..]));
    assert_eq!(fields.number(&langs), Ok(7));
    assert_eq!(fields.number(&langs), Err(malformed(&langs)));
    // 声明的长度比剩下的多。
    assert_eq!(
        Fields(&[9, 0, 0, 0, 1]).bytes(&langs),
        Err(malformed(&langs))
    );
}
