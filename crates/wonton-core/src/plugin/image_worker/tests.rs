//! 子进程的退出状态怎么算。真的起子进程的测试在 `tests/image_worker.rs`（要 `wonton-core` 可执行文件）。

use super::*;

/// wait(2) 的状态编码：被信号结束时低 7 位是信号，正常退出时退出码在第 8 到 15 位。
fn exited(code: i32) -> ExitStatus {
    ExitStatus::from_raw(code << 8)
}

fn signaled(signal: Signal) -> ExitStatus {
    ExitStatus::from_raw(signal.as_raw())
}

fn refused(status: ExitStatus, stderr: &str) -> String {
    match outcome(status, Vec::new(), stderr) {
        Err(ImageError::Refused(reason)) => reason,
        other => panic!("{other:?}"),
    }
}

#[test]
fn exit_statuses_tell_the_image_apart_from_the_worker() {
    assert_eq!(outcome(exited(0), b"out".to_vec(), ""), Ok(b"out".to_vec()));
    assert_eq!(refused(exited(1), "图标要是正方形\n"), "图标要是正方形");
    // 起不来、设不了上限：不是图片的问题，不该记成这张图处理不了。
    assert_eq!(
        outcome(exited(2), Vec::new(), "设不了资源上限：EPERM"),
        Err(ImageError::Unavailable("设不了资源上限：EPERM".to_owned()))
    );
}

#[test]
fn crashes_and_limits_are_reported_as_unprocessable_with_the_cause() {
    assert!(refused(exited(101), "thread 'main' panicked at …").contains("崩溃"));
    assert!(refused(signaled(Signal::XCPU), "").contains("CPU 时间"));
    let oom = refused(
        signaled(Signal::ABORT),
        "memory allocation of 1073741824 bytes failed",
    );
    assert!(oom.contains("内存超过 512 MiB"), "{oom}");
    assert!(refused(signaled(Signal::KILL), "").contains("内存不够"));
    let term = Signal::TERM.as_raw();
    assert!(refused(signaled(Signal::TERM), "").contains(&format!("信号 {term}")));
}

#[test]
fn malformed_output_is_the_workers_fault() {
    let mut out = Vec::new();
    put(&mut out, b"jpeg");
    put(&mut out, &7u32.to_le_bytes());
    let mut fields = Fields(&out);
    assert_eq!(fields.bytes(), Ok(&b"jpeg"[..]));
    assert_eq!(fields.number(), Ok(7));
    assert_eq!(fields.number(), Err(malformed()));
    // 声明的长度比剩下的多。
    assert_eq!(Fields(&[9, 0, 0, 0, 1]).bytes(), Err(malformed()));
}
