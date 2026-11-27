use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use wonton_onboard::board::Board;
use wonton_onboard::verify::{self, GhApi};
use wonton_onboard::{cleanup, finish, watch};

#[derive(Parser)]
#[command(about = "收尾工具：verify 批次、owner/recover、close-guard 与恢复")]
struct Cli {
    /// 任务工作目录；默认当前目录。
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 跑一批 PR 的 verify 并收尾
    Finish {
        /// 收掉死进程留下的 in_progress / 收尾记录
        #[arg(long)]
        recover: bool,
        /// 逗号分隔的 PR 号
        #[arg(long, value_delimiter = ',')]
        prs: Vec<u64>,
    },
    /// 对已有批次只跑明细
    Verify {
        #[arg(long, value_delimiter = ',')]
        prs: Vec<u64>,
    },
    /// 放弃当前作业，保留记录给 recover
    Cancel,
    /// 显式恢复已死作业并继续收尾
    Recover,
    /// 每 N 分钟自动恢复一次
    Watch {
        #[arg(long, default_value = "5m")]
        every: String,
        /// 只跑一轮
        #[arg(long)]
        once: bool,
    },
    /// 对比 live 记录和收尾记录，owner 优先
    CompareNotes,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("onboard: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = cli
        .root
        .unwrap_or_else(|| std::env::current_dir().expect("当前目录"));
    for path in cleanup::sweep_roots(&cleanup::roots_from_env(&root))? {
        eprintln!("清扫 {}", path.display());
    }
    let api = GhApi::new(&root);

    match cli.command {
        Command::Finish { recover, prs } => {
            let request = finish::FinishRequest {
                recover,
                prs: prs.into_iter().collect(),
            };
            let outcome = finish::finish(&root, &request, &api, finish::LOCK_TIMEOUT)?;
            report(&outcome);
            if outcome == finish::Outcome::JsonPending {
                std::process::exit(1);
            }
        }
        Command::Verify { prs } => verify_only(&root, prs, &api)?,
        Command::Cancel => {
            let kept = finish::cancel(&root)?;
            println!(
                "{}",
                if kept {
                    "记录保留，job 未动"
                } else {
                    "job 已清"
                }
            );
        }
        Command::Recover => report(&finish::recover(&root, &api)?),
        Command::Watch { every, once } => watch_loop(&root, &every, once, &api)?,
        Command::CompareNotes => match watch::compare_notes(&root)? {
            watch::CompareOutcome::Busy => println!("busy: owner alive"),
            watch::CompareOutcome::Clean => println!("clean"),
            watch::CompareOutcome::Reconciled(reasons) => {
                println!("reconciled: {}", reasons.join("; "));
            }
        },
    }
    Ok(())
}

fn verify_only(root: &Path, prs: Vec<u64>, api: &dyn verify::Api) -> Result<()> {
    let mut board = Board::open_existing(root)?;
    let prs: BTreeSet<u64> = if prs.is_empty() {
        board
            .current_batch()
            .map(|(_, batch)| batch.prs.clone())
            .unwrap_or_default()
    } else {
        prs.into_iter().collect()
    };
    if prs.is_empty() {
        bail!("没有可 verify 的 PR");
    }
    let prs: Vec<u64> = prs.into_iter().collect();
    verify::verify_cycle(&mut board, &prs, api)?;
    println!("verify 完成：{} 个 PR", prs.len());
    Ok(())
}

fn watch_loop(root: &Path, every: &str, once: bool, api: &dyn verify::Api) -> Result<()> {
    let interval = parse_duration(every)?;
    loop {
        println!("{}", watch::autopilot(root, api)?);
        if once {
            return Ok(());
        }
        std::thread::sleep(interval);
    }
}

fn parse_duration(text: &str) -> Result<Duration> {
    let split = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: u64 = number
        .parse()
        .with_context(|| format!("间隔不合法：{text}"))?;
    if number == 0 {
        return Ok(Duration::from_secs(300));
    }
    match unit {
        "" | "s" => Ok(Duration::from_secs(number)),
        "m" => Ok(Duration::from_secs(number * 60)),
        "h" => Ok(Duration::from_secs(number * 3_600)),
        _ => bail!("间隔单位只认 s/m/h：{text}"),
    }
}

fn report(outcome: &finish::Outcome) {
    match outcome {
        finish::Outcome::Busy => println!("busy: owner alive"),
        finish::Outcome::JsonPending => println!("未收尾记录还在：先跑 finish --recover"),
        finish::Outcome::ConflictAborted(reasons) => {
            println!(
                "冲突已收敛：{}；用 finish --recover 续跑",
                reasons.join("; ")
            );
        }
        finish::Outcome::Closed {
            cycle,
            within_cycle,
        } => {
            println!(
                "已收尾 {cycle}（{}）",
                if *within_cycle {
                    "同 cycle 重试"
                } else {
                    "新 cycle"
                }
            );
        }
    }
}
