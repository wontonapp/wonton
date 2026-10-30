//! `wonton-core plugin check <插件包目录> [--official]`：插件作者发布前的检查，规则见 `wonton_core::plugin::check`。
//! 通过时退出码 0；有不合格的地方时 1；用法不对时 2。

use std::path::Path;
use std::process::ExitCode;

use wonton_core::plugin::check::{self, Report};
use wonton_core::plugin::image_worker::ImageWorker;

const USAGE: &str = "用法：wonton-core plugin check <插件包目录> [--official]";

pub async fn run(args: &[String]) -> ExitCode {
    let Some((command, rest)) = args.split_first() else {
        return usage();
    };
    if command != "check" {
        return usage();
    }
    let official = rest.iter().any(|a| a == "--official");
    let dirs: Vec<&String> = rest.iter().filter(|a| *a != "--official").collect();
    let [dir] = dirs[..] else {
        return usage();
    };
    if dir.starts_with('-') {
        return usage();
    }
    let images = match ImageWorker::current() {
        Ok(images) => images,
        Err(e) => {
            eprintln!("找不到 wonton-core 自己的可执行文件，处理不了图片：{e}");
            return ExitCode::FAILURE;
        }
    };
    match check::check(Path::new(dir), official, &images).await {
        Ok(report) => print(&report),
        Err(e) => {
            eprintln!("不合格：{e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn print(report: &Report) -> ExitCode {
    println!("插件 {} {}", report.id, report.version);
    println!("文件：");
    for (path, size) in &report.files {
        println!("  {path}（{}）", human_size(*size));
    }
    if !report.warnings.is_empty() {
        println!("提醒：");
        for warning in &report.warnings {
            println!("  - {warning}");
        }
    }
    println!("插件包摘要：{}", report.digest);
    if report.problems.is_empty() {
        println!("通过");
        return ExitCode::SUCCESS;
    }
    eprintln!("不合格：");
    for problem in &report.problems {
        eprintln!("  - {problem}");
    }
    ExitCode::FAILURE
}

fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}
