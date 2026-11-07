//! `wonton-core plugin`：插件作者用的命令，在发布的核心镜像里就有。
//!
//! - `plugin check <插件包目录> [--official]`：发布前的检查，规则见 `wonton_core::plugin::check`。
//! - `plugin pack <插件包目录>`：把插件包（compose 和它引用的文件）打成 tar 写到标准输出，开发工具拿它上传安装；
//!   README、截图这些不属于插件包的文件不在里面。
//! - `plugin index-entry <插件包目录> <github:owner/repo> <提交号> [--index <索引目录>]`：插件包在那个提交上的官方索引
//!   条目（插件规范的“官方索引”）。不带 `--index` 时把条目的 JSON 写到标准输出；带了就写进那个本地的索引目录，
//!   文件按 sha256 放进 `blobs/`，同一个来源的旧条目换掉。插件包目录要是那个提交的检出：核心不连 Git，不核对。
//! - `plugin fetch-locked <锁文件> <目标目录>`：构建镜像时按提交取插件包，核对锁文件里的来源、提交和摘要。
//! - `plugin host [插件包目录]`：这台主机实际有什么（内存、系统、架构、显卡），JSON 写到标准输出，读不出来的项写明原因。
//!   和判断插件的“要求”时用的是同一份。给了插件包时还有它的要求逐条的判断和装不了的原因。
//! - `plugin slice-unit`：根据宿主 `/proc/meminfo` 输出插件 systemd slice unit。读不到总内存时退出码 1，并把原因写到标准错误。
//! - `plugin check-index <index.json>`：官方索引里的每一项这个版本的核心都读得懂，CI 用它防止改了清单写法后商店变空。
//!
//! 通过时退出码 0；插件包不合格时 1；用法不对时 2。

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use wonton_core::plugin::PluginPackage;
use wonton_core::plugin::check::{self, Report};
use wonton_core::plugin::docker::Engine;
use wonton_core::plugin::gpu::HostPaths;
use wonton_core::plugin::image_worker::ImageWorker;
use wonton_core::plugin::languages::Languages;
use wonton_core::plugin::names::Instance;
use wonton_core::plugin::storefront::index::{Index, IndexEntry};
use wonton_core::plugin::{host_check, host_facts};

mod locked;

const USAGE: &str = "用法：wonton-core plugin check <插件包目录> [--official]\n      \
                     wonton-core plugin pack <插件包目录> > 插件包.tar\n      \
                     wonton-core plugin index-entry <插件包目录> <github:owner/repo> <提交号> [--index <索引目录>]\n      \
                     wonton-core plugin fetch-locked <锁文件> <目标目录>\n      \
                     wonton-core plugin host [插件包目录]\n      \
                     wonton-core plugin slice-unit\n      \
                     wonton-core plugin check-index <index.json>";

pub async fn run(args: &[String]) -> ExitCode {
    match args {
        [command, rest @ ..] if command == "check" => check_dir(rest).await,
        [command, dir] if command == "pack" && !dir.starts_with('-') => pack(Path::new(dir)),
        [command, rest @ ..] if command == "index-entry" => index_entry(rest),
        [command, lock, destination] if command == "fetch-locked" => {
            locked::run(Path::new(lock), Path::new(destination)).await
        }
        [command] if command == "host" => host(None).await,
        [command, dir] if command == "host" && !dir.starts_with('-') => {
            host(Some(Path::new(dir))).await
        }
        [command] if command == "slice-unit" => slice_unit(),
        [command, path] if command == "check-index" && !path.starts_with('-') => {
            check_index(Path::new(path))
        }
        _ => usage(),
    }
}

fn slice_unit() -> ExitCode {
    let text = match std::fs::read_to_string("/proc/meminfo") {
        Ok(text) => text,
        Err(error) => {
            eprintln!("读不了 /proc/meminfo：{error}");
            return ExitCode::FAILURE;
        }
    };
    match wonton_core::plugin::limits::slice_unit_from_meminfo(&text) {
        Ok(unit) => {
            print!("{unit}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("生成插件 slice 失败：{error}");
            ExitCode::FAILURE
        }
    }
}

async fn host(dir: Option<&Path>) -> ExitCode {
    let package = match dir
        .map(|dir| PluginPackage::load_dir(dir, &Languages::default()))
        .transpose()
    {
        Ok(package) => package,
        Err(e) => {
            eprintln!("不合格：{e}");
            return ExitCode::FAILURE;
        }
    };
    let instance = match Instance::parse(std::env::var("WONTON_INSTANCE").ok().as_deref()) {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("WONTON_INSTANCE 不合法：{e}");
            return ExitCode::FAILURE;
        }
    };
    let dev = std::env::var_os("WONTON_HOST_DEV").map_or_else(|| "/dev".into(), PathBuf::from);
    let paths = HostPaths::new(Some(dev));
    let engine = Engine::connect();
    let docker_info = match &engine {
        Ok(engine) => engine.host_info().await.map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    };
    let limits = docker_info.as_ref().map_or_else(
        |_| serde_json::json!({
            "MemoryLimit": null,
            "SwapLimit": null,
            "CPUShares": null,
            "PidsLimit": null,
            "WONTON_PLUGIN_SLICE": std::env::var("WONTON_PLUGIN_SLICE").ok().filter(|value| !value.is_empty()),
            "CgroupDriver": null,
            "CgroupVersion": null,
        }),
        |info| {
            serde_json::json!({
                "MemoryLimit": info.memory_limit,
                "SwapLimit": info.swap_limit,
                "CPUShares": info.cpu_shares,
                "PidsLimit": info.pids_limit,
                "WONTON_PLUGIN_SLICE": std::env::var("WONTON_PLUGIN_SLICE").ok().filter(|value| !value.is_empty()),
                "CgroupDriver": info.cgroup_driver,
                "CgroupVersion": info.cgroup_version,
            })
        },
    );
    let langs = Languages::default();
    let mut facts = host_facts::read(&paths, docker_info, &langs);
    if let Ok(engine) = &engine {
        let core = std::env::var("WONTON_CORE_CONTAINER").ok();
        host_facts::NvidiaMemory::default()
            .fill(&mut facts, engine, core.as_deref(), &instance, &langs)
            .await;
    }
    let mut report = serde_json::json!({ "host": facts, "limits": limits });
    if let Some(package) = &package {
        let manifest = &package.manifest;
        let checks = manifest
            .requirements
            .as_ref()
            .map(|r| host_check::judge(r, &facts, &langs))
            .unwrap_or_default();
        let lines: Vec<_> = checks.into_iter().map(|check| check.line).collect();
        report["requirements"] = serde_json::json!(lines);
        report["blocked"] = serde_json::json!(
            host_check::refusal(manifest, &facts, &langs).map(|refusal| refusal.to_string())
        );
    }
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("写不出主机情况：{e}");
            ExitCode::FAILURE
        }
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
    let package = match PluginPackage::load_dir(dir, &Languages::default()) {
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

fn check_index(path: &Path) -> ExitCode {
    let index = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|json| Index::parse(&json, &Default::default()).map_err(|e| e.to_string()));
    let index = match index {
        Ok(index) => index,
        Err(e) => {
            eprintln!("读不了 {}：{e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    for plugin in &index.skipped {
        eprintln!("读不懂 {}：{}", plugin.source, plugin.reason);
    }
    if !index.skipped.is_empty() {
        return ExitCode::FAILURE;
    }
    println!("{} 个插件都读得懂", index.plugins.len());
    ExitCode::SUCCESS
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
    let package = match PluginPackage::load_dir(Path::new(dir), &Languages::default()) {
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
