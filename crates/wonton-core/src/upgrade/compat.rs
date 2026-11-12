//! 升级前的插件兼容检查（ADR-0085）：当前核心把已装记录物化到数据目录，在目标镜像的一次性
//! 容器里跑 `plugin compat`，只报"新失效"——目标读不懂、当前核心读得懂的。现在就异常的不重复列，
//! 系统插件跟着核心镜像走不查；查不成如实给原因，升级照常可以发起，不拦。

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use wonton_protocol::{
    CoreUpgradeBreakingPlugin, CoreUpgradeBreakingReason, CoreUpgradeCompat,
    CoreUpgradeCompatCategory, CoreUpgradeFailure as Failure,
};

use super::host::{Host, Mount, OneShot, OneShotOutput};
use super::state::problem;
use crate::l10n;
use crate::plugin::compat::{self, Verdict};
use crate::plugin::languages::Languages;
use crate::plugin::manifest::PluginKind;
use crate::plugin::package::PluginPackage;
use crate::plugin::package_files::COMPOSE_FILE;
use crate::plugin::store::{InstalledRecord, Store};

/// 一次 compat 的限时（ADR-0085）。
const TIMEOUT: Duration = Duration::from_secs(60);
/// compat 容器的名字：`run_once` 开始和结束都会清掉它。
const CONTAINER: &str = "wonton-compat";

/// 查一遍：结果跟着 digest 走，digest 变了下次检查重跑。
pub async fn check(
    host: &dyn Host,
    reference: &str,
    digest: &str,
    data_dir: &Path,
    langs: &Languages,
) -> CoreUpgradeCompat {
    let packages = match readable(data_dir, langs) {
        Ok(packages) => packages,
        Err(error) => {
            return failed(l10n::text(
                langs,
                "upgrade-compat-records-unreadable",
                &[("error", error.into())],
            ));
        }
    };
    if packages.is_empty() {
        return CoreUpgradeCompat::Ok;
    }
    // 物化目录每次检查先删上一次的，读完再删；放 `upgrade/` 下顺带被备份排除。
    let root = data_dir
        .join("core")
        .join("upgrade")
        .join("compat")
        .join(digest.replace([':', '/'], "-"));
    let _ = std::fs::remove_dir_all(root.parent().expect("compat 目录有父级"));
    let dirs = match materialize(&root, &packages) {
        Ok(dirs) => dirs,
        Err(error) => {
            return failed(l10n::text(
                langs,
                "upgrade-compat-materialize-failed",
                &[("error", error.into())],
            ));
        }
    };
    let ids: BTreeSet<String> = packages.iter().map(|(id, _)| id.clone()).collect();
    let mut cmd = vec!["plugin".to_owned(), "compat".to_owned()];
    cmd.extend(dirs);
    let job = OneShot {
        name: CONTAINER.to_owned(),
        image: reference.to_owned(),
        entrypoint: None,
        cmd,
        mounts: vec![Mount::same_path(&root, true)],
        timeout: TIMEOUT,
    };
    let result = match host.run_once(&job).await {
        Err(error) => failed(l10n::text(
            langs,
            "upgrade-compat-run-failed",
            &[
                ("image", reference.into()),
                ("error", error.to_string().into()),
            ],
        )),
        Ok(output) => interpret(&output, reference, &ids, langs),
    };
    let _ = std::fs::remove_dir_all(root.parent().expect("compat 目录有父级"));
    result
}

/// 当前核心读得懂的非系统插件：目标裁决 breaking 时，只有它们算"新失效"。
fn readable(data_dir: &Path, langs: &Languages) -> Result<Vec<(String, InstalledRecord)>, String> {
    let store = Store::open(data_dir.join("core").join("plugins")).map_err(|e| e.to_string())?;
    let mut packages = Vec::new();
    for file in store.load_all().map_err(|e| e.to_string())? {
        // 现在就异常的（当前核心都读不懂）不重复列：插件页已经显示它们，升级不改变它们的处境。
        let Ok(record) = file.record else {
            continue;
        };
        let Ok(package) = PluginPackage::with_files(&record.source, record.files.clone(), langs)
        else {
            continue;
        };
        // 系统插件跟着核心镜像走，新核心放行后自己会升级它们（ADR-0025）。
        if package.manifest.kind == PluginKind::System {
            continue;
        }
        packages.push((record.id.to_string(), record));
    }
    Ok(packages)
}

/// 把每份已装记录写成插件包目录：compose 原文加它引用的文件，compat 子命令读同一套。
fn materialize(root: &Path, packages: &[(String, InstalledRecord)]) -> Result<Vec<String>, String> {
    let mut dirs = Vec::new();
    for (id, record) in packages {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(COMPOSE_FILE), &record.source).map_err(|e| e.to_string())?;
        for (path, data) in &record.files {
            let target = dir.join(path);
            // 记录里的路径是安装时校验过的相对路径；材料化再挡一次目录穿越。
            if !target.starts_with(&dir) {
                return Err(format!("{id}：{path}"));
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&target, data).map_err(|e| e.to_string())?;
        }
        dirs.push(dir.display().to_string());
    }
    Ok(dirs)
}

/// 容器的裁决转成接口状态：退出码 0＝裁决完成（有失效的插件也是 0），
/// 2＝用法不对＝目标镜像太老没有这个子命令，其余＝命令自身出错。
/// 公开给契约测试：真实子命令进程和替身输出都从这里过（ADR-0085 的缝一）。
pub fn interpret(
    output: &OneShotOutput,
    reference: &str,
    ids: &BTreeSet<String>,
    langs: &Languages,
) -> CoreUpgradeCompat {
    match output.exit_code {
        2 => return CoreUpgradeCompat::Unavailable,
        0 => {}
        code => {
            return failed(l10n::text(
                langs,
                "upgrade-compat-command-failed",
                &[
                    ("image", reference.into()),
                    ("code", code.to_string().into()),
                    ("error", output.stderr.trim().into()),
                ],
            ));
        }
    }
    let parsed = match serde_json::from_str::<compat::Output>(&output.stdout) {
        Ok(parsed) => parsed,
        Err(error) => {
            return failed(l10n::text(
                langs,
                "upgrade-compat-output-unreadable",
                &[("error", error.to_string().into())],
            ));
        }
    };
    let plugins: Vec<CoreUpgradeBreakingPlugin> = parsed
        .plugins
        .into_iter()
        // 目标报告 breaking、且 id 在这次送查的清单里，才算新失效。
        .filter(|verdict| verdict.verdict == Verdict::Breaking && ids.contains(&verdict.id))
        .map(|verdict| CoreUpgradeBreakingPlugin {
            id: verdict.id,
            reasons: verdict
                .reasons
                .into_iter()
                .map(|reason| CoreUpgradeBreakingReason {
                    category: category(&reason.category),
                    raw: reason.raw,
                })
                .collect(),
        })
        .collect();
    if plugins.is_empty() {
        CoreUpgradeCompat::Ok
    } else {
        CoreUpgradeCompat::Breaking { plugins }
    }
}

/// 目标给的类别字符串转接口枚举：认得的逐字对照，其余（known other 和未来类别）落 Other。
fn category(text: &str) -> CoreUpgradeCompatCategory {
    match text {
        compat::SPEC_UNSUPPORTED => CoreUpgradeCompatCategory::SpecUnsupported,
        compat::UNKNOWN_FEATURE => CoreUpgradeCompatCategory::UnknownFeature,
        compat::CAPABILITY_MISSING => CoreUpgradeCompatCategory::CapabilityMissing,
        _ => CoreUpgradeCompatCategory::Other,
    }
}

fn failed(message: String) -> CoreUpgradeCompat {
    CoreUpgradeCompat::Failed {
        problem: problem(Failure::Upstream, message),
    }
}
