//! `wonton-core plugin`：插件作者用的命令，在发布的核心镜像里就有。
//!
//! - `plugin check <插件包目录> [--official]`：发布前的检查，规则见 `wonton_core::plugin::check`。
//! - `plugin pack <插件包目录>`：把插件包（compose 和它引用的文件）打成 tar 写到标准输出，开发工具拿它上传安装；
//!   README、截图这些不属于插件包的文件不在里面。
//! - `plugin index-entry <插件包目录> <github:owner/repo> <提交号> [--index <索引目录>]`：插件包在那个提交上的官方索引
//!   条目（插件规范的“官方索引”）。不带 `--index` 时把条目的 JSON 写到标准输出；带了就写进那个本地的索引目录，
//!   文件按 sha256 放进 `blobs/`，同一个来源的旧条目换掉。插件包目录要是那个提交的检出：核心不连 Git，不核对。
//!
//! 通过时退出码 0；插件包不合格时 1；用法不对时 2。

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use wonton_core::plugin::PluginPackage;
use wonton_core::plugin::check::{self, Report};
use wonton_core::plugin::image_worker::ImageWorker;
use wonton_core::plugin::storefront::index::IndexEntry;

const USAGE: &str = "用法：wonton-core plugin check <插件包目录> [--official]\n      \
                     wonton-core plugin pack <插件包目录> > 插件包.tar\n      \
                     wonton-core plugin index-entry <插件包目录> <github:owner/repo> <提交号> [--index <索引目录>]";

pub async fn run(args: &[String]) -> ExitCode {
    match args {
        [command, rest @ ..] if command == "check" => check_dir(rest).await,
        [command, dir] if command == "pack" && !dir.starts_with('-') => pack(Path::new(dir)),
        [command, rest @ ..] if command == "index-entry" => index_entry(rest),
        _ => usage(),
    }
}

async fn check_dir(args: &[String]) -> ExitCode {
    let official = args.iter().any(|a| a == "--official");
    let dirs: Vec<&String> = args.iter().filter(|a| *a != "--official").collect();
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

fn pack(dir: &Path) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    if stdout.is_terminal() {
        eprintln!("插件包是二进制的 tar：把标准输出重定向到文件或管道");
        return usage();
    }
    let package = match PluginPackage::load_dir(dir) {
        Ok(package) => package,
        Err(e) => {
            eprintln!("不合格：{e}");
            return ExitCode::FAILURE;
        }
    };
    match stdout
        .write_all(&package.archive())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("写标准输出失败：{e}");
            ExitCode::FAILURE
        }
    }
}

fn index_entry(args: &[String]) -> ExitCode {
    let (positional, index) = match args {
        [rest @ .., flag, index] if flag == "--index" => (rest, Some(Path::new(index))),
        rest => (rest, None),
    };
    let [dir, source, commit] = positional else {
        return usage();
    };
    if [dir, source, commit].iter().any(|a| a.starts_with('-')) {
        return usage();
    }
    let package = match PluginPackage::load_dir(Path::new(dir)) {
        Ok(package) => package,
        Err(e) => {
            eprintln!("不合格：{e}");
            return ExitCode::FAILURE;
        }
    };
    let entry = match IndexEntry::of(&package, source, commit) {
        Ok(entry) => entry,
        Err(e) => {
            eprintln!("生成不了索引条目：{e}");
            return ExitCode::FAILURE;
        }
    };
    let written = match index {
        Some(index) => entry.write_to(index, &package),
        None => {
            let json = serde_json::to_string_pretty(&entry).expect("索引条目能写成 JSON");
            println!("{json}");
            Ok(())
        }
    };
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
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
