//! 复现 #723：新客户端核心配上一台旧服务器，把首次配对和引导的真实结果打出来。不改任何东西，只报告。
//!
//! ```text
//! cargo run -p wonton-client --example old-server-e2e -- <数据目录> <配对码>
//! ```

// 同 wonton-client：matrix-sdk 的 future 嵌套很深。
#![recursion_limit = "256"]

use std::process::ExitCode;
use std::time::{Duration, Instant};

use wonton_client::{Action, ClientCore, HomeStatus, Network, OnboardingStep, SecretString};

const CONNECT: Duration = Duration::from_secs(60);
const QUIET: Duration = Duration::from_secs(30);

/// 跳过能跳的步走到最后一页并点完成，打印每一步和最后的提交状态。
async fn walk(app: &std::sync::Arc<ClientCore>) -> ExitCode {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let view = app.onboarding();
        println!("当前步骤：{:?}", view.step);
        match view.step {
            Some(OnboardingStep::Review) => break,
            None => {
                eprintln!("没有步骤：{view:?}");
                return ExitCode::from(1);
            }
            _ => {}
        }
        let action = match view.step {
            Some(OnboardingStep::Finished) => break,
            Some(_) => Action::SkipOnboardingStep,
            None => unreachable!(),
        };
        if let Err(error) = app.dispatch(action) {
            // 最后一页不能跳：直接点完成。
            if matches!(view.step, Some(OnboardingStep::Review)) {
                break;
            }
            eprintln!("走不到下一步：{error}");
            return ExitCode::from(1);
        }
        if Instant::now() > deadline {
            eprintln!("走步骤超时：{view:?}");
            return ExitCode::from(1);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    // 预览要取（这里只有默认选择），等最后的项都定下来。
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let view = app.onboarding();
        let ready = view
            .review
            .items
            .iter()
            .all(|item| item.state != wonton_client::OnboardingItemState::Preparing);
        if ready {
            break;
        }
        if Instant::now() > deadline {
            eprintln!("等预览超时：{view:?}");
            return ExitCode::from(1);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    if let Err(error) = app.dispatch(Action::ContinueOnboarding) {
        eprintln!("点完成失败：{error}");
        return ExitCode::from(1);
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let view = app.onboarding();
        if view.review.commit != wonton_client::OnboardingCommit::NotStarted {
            println!(
                "提交：{:?}，项：{:?}",
                view.review.commit,
                view.review
                    .items
                    .iter()
                    .map(|item| (item.id.clone(), item.state.clone()))
                    .collect::<Vec<_>>()
            );
        }
        if view.review.commit == wonton_client::OnboardingCommit::Done {
            println!("完成后的步骤：{:?}", view.step);
            return ExitCode::SUCCESS;
        }
        if Instant::now() > deadline {
            eprintln!("等完成超时：{view:?}");
            return ExitCode::from(1);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|arg| !arg.starts_with("--")).collect();
    let [dir, code] = positional.as_slice() else {
        eprintln!("用法：old-server-e2e <数据目录> <配对码>");
        return ExitCode::from(2);
    };
    let app = match ClientCore::with_network(dir, Network::DirectOnly) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("创建核心失败：{error}");
            return ExitCode::from(1);
        }
    };
    if let Err(error) = app.dispatch(Action::PairHome {
        code: SecretString::new((*code).clone()),
        device_name: "compat-e2e".to_owned(),
    }) {
        eprintln!("配对码不合法：{error}");
        return ExitCode::from(1);
    }
    let deadline = Instant::now() + CONNECT;
    loop {
        let home = app.home();
        match home.status {
            HomeStatus::Connected => break,
            HomeStatus::Disconnected {
                ref reason,
                ref message,
            } => {
                eprintln!("连接失败：{reason:?} {message}");
                return ExitCode::from(1);
            }
            _ => {}
        }
        if Instant::now() > deadline {
            eprintln!("等连接超时：{home:?}");
            return ExitCode::from(1);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!("已配对并连上家里的服务器");

    // 连上以后升级状态会取一次：那是服务器版本的来源。
    let deadline = Instant::now() + QUIET;
    loop {
        let view = app.onboarding();
        let version = app
            .plugins()
            .core_upgrade
            .status
            .and_then(|status| status.current.version);
        if !view.loading && view.step.is_some() {
            println!("服务器版本：{version:?}");
            println!("引导步骤：{:?}", view.step);
            println!("步骤计数：{:?}", view.nav);
            println!("引导问题：{:?}", view.problem);
            println!("服务器太旧：{:?}", view.too_old);
            println!(
                "资料接口：{:?}",
                app.me()
                    .await
                    .map(|me| me.display_name)
                    .map_err(|e| format!("{e:?}"))
            );
            if std::env::args().any(|arg| arg == "--walk") {
                return walk(&app).await;
            }
            return ExitCode::SUCCESS;
        }
        if Instant::now() > deadline {
            eprintln!("等引导超时：{view:?}");
            return ExitCode::from(1);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
