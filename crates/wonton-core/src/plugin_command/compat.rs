//! `plugin compat <插件包目录>…`（ADR-0085）：这份核心读不读得懂每份插件包，裁决 JSON 写标准输出。
//! 目标镜像在升级前自证支持范围用，不启动任何容器、不连 Docker。

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use wonton_core::plugin::compat;
use wonton_core::plugin::languages::Languages;

pub(super) fn run(args: &[String]) -> ExitCode {
    if args.is_empty() || args.iter().any(|arg| arg.starts_with('-')) {
        return super::usage();
    }
    let dirs: Vec<&Path> = args.iter().map(Path::new).collect();
    let output = compat::output(&dirs, &Languages::default());
    let Ok(json) = serde_json::to_string(&output) else {
        eprintln!("裁决 JSON 序列化不了");
        return ExitCode::FAILURE;
    };
    let stdout = std::io::stdout();
    let written = stdout
        .lock()
        .write_all(json.as_bytes())
        .and_then(|_| writeln!(stdout.lock()));
    if let Err(error) = written {
        eprintln!("写不了标准输出：{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
