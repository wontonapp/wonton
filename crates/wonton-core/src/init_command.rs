//! 安装入口与终端联邦黑名单命令。

use super::lifecycle_commands::check_owner_before_install;
use anyhow::{Context, bail};
use wonton_core::matrix::FederationBlocks;
use wonton_core::plugin::docker::Engine;
/// 安装顺序：注册并记下名字、签证书、写配置，最后才由安装脚本启动 Tuwunel。
pub(super) async fn init(args: &[String]) -> anyhow::Result<()> {
    // 第二个参数是基础域名：旧版本的升级器照这个格式调新镜像的 `init`，所以留着，`init` 不再用它。
    let [dir, _base_domain] = args else {
        bail!("用法：wonton-core init <安装目录> <基础域名>");
    };
    check_owner_before_install()?;
    let dir = std::path::Path::new(dir);
    let data_dir = std::env::var("WONTON_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|_| read_installed_data_dir(dir))?;
    let settings = wonton_core::entry_settings::EntrySettings::select(dir, &data_dir).await?;
    if let Some(settings) = &settings {
        if !dir.join("entry/certs/site.pem").is_file() {
            let domain = wonton_core::clients::domain::Domain::new(
                dir.to_owned(),
                data_dir.clone(),
                Engine::connect()?,
                wonton_core::plugin::names::Instance::default(),
                None,
            );
            domain.issue_now().await?;
        }
        wonton_core::entry_compose::write(dir, settings)?;
    }
    let requested = settings.as_ref().map(|settings| settings.domain.clone());
    let report = wonton_core::install::init(dir, requested.as_deref())?;
    for path in &report.created {
        println!("已生成 {}", path.display());
    }
    for path in &report.kept {
        println!("沿用 {}", path.display());
    }
    Ok(())
}

fn read_installed_data_dir(dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let env = std::fs::read_to_string(dir.join(".env"))?;
    env.lines()
        .find_map(|line| line.strip_prefix("WONTON_DATA_DIR="))
        .map(std::path::PathBuf::from)
        .context("安装需要 WONTON_DATA_DIR")
}

pub(super) fn federation_block(args: &[String]) -> anyhow::Result<()> {
    let [dir, action, server] = args else {
        bail!("用法：wonton-core federation-block <数据目录> <add|remove> <服务器名>");
    };
    let blocked = match action.as_str() {
        "add" => true,
        "remove" => false,
        _ => bail!("操作只能是 add 或 remove"),
    };
    FederationBlocks::open(std::path::Path::new(dir).join("core/federation-blocks.json"))?
        .set(server, blocked)
}
