//! 容器上的渲染摘要和这次渲染的对不上（核心升级改了注入的值，或者旧容器没有摘要）就照安装的路径重建，
//! 卷和数据目录不动（ADR-0063）。
//!
//! 摘要按渲染结果算，打在容器的 `wonton.render` 标签上，所以打标签必须在渲染的最后一步：镜像引用之后会在拉取时
//! 换成拉取后的写法（可能带镜像加速的地址和摘要），开了代理时健康检查还会换成镜像自带的，两者都由渲染结果里
//! 已有的东西决定，重建和核对都按同一份渲染比，不受影响。

use sha2::{Digest, Sha256};

use super::health_gate::Gate;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::package::PluginPackage;
use super::render::{ContainerPlan, Deployment, render};
use super::store::InstalledRecord;

/// 渲染结果的摘要。
pub(super) const LABEL_RENDER: &str = "wonton.render";

/// 建容器用到的字段的摘要。标签自己不参与。
pub(super) fn digest(container: &ContainerPlan) -> String {
    let mut body = container.body.clone();
    if let Some(labels) = &mut body.labels {
        labels.remove(LABEL_RENDER);
    }
    let files: Vec<serde_json::Value> = container
        .files
        .iter()
        .map(|file| {
            serde_json::json!({
                "target": file.target.display().to_string(),
                "uid": file.uid,
                "gid": file.gid,
                "mode": file.mode,
                "data": file.data,
            })
        })
        .collect();
    // ACP SDK 开启 preserve_order，摘要必须显式排序，不能依赖 serde_json 的编译特性。
    let mut value = serde_json::json!({
        "name": container.name,
        "body": body,
        "files": files,
        "store_dir": container.store_dir,
    });
    value.sort_all_objects();
    let json = serde_json::to_string(&value).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(json.as_bytes()))
}

/// 算好摘要并打上标签。
pub(super) fn stamp(container: &mut ContainerPlan) {
    let digest = digest(container);
    container
        .body
        .labels
        .get_or_insert_with(Default::default)
        .insert(LABEL_RENDER.to_owned(), digest);
}

struct Drift {
    /// 有容器对不上、没有标签或者不在了。
    drifted: bool,
    /// 还在运行的容器数：全停着的插件由启动流程决定动不动。
    running: usize,
}

impl PluginManager {
    /// 渲染和容器上的摘要对不上就重建全部容器；`when_stopped` 时全停着的插件也重建。返回重建过没有。
    /// 失败照实往外报，不回退成用旧容器。`gate` 是等健康的时限：开机恢复和容器监视串行核对，用固定的
    /// 安装时限，别人触发的重建按插件声明。
    pub(super) async fn recreate_on_drift(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        langs: &Languages,
        when_stopped: bool,
        gate: Gate,
    ) -> Result<bool, InstallError> {
        // 局域网端口照检测出来的地址渲染，和局域网巡检、按代理/模型重建走同一条路。
        let lan = self.lan_grant(package, record.lan_webui, langs).await?;
        let mut ctx = self.rebuild_context(package, record, lan, None).await?;
        self.fill_bot_vars(package, &mut ctx, langs).await?;
        let plan = render(package, &ctx, langs)?;
        let drift = self.drift_of(&plan).await?;
        if !drift.drifted || (drift.running == 0 && !when_stopped) {
            return Ok(false);
        }
        tracing::info!(plugin = %package.manifest.id, "插件容器的渲染变了，按安装的路径重建");
        self.replace_containers(&ctx, plan, gate).await?;
        Ok(true)
    }

    async fn drift_of(&self, plan: &Deployment) -> Result<Drift, InstallError> {
        let mut drift = Drift {
            drifted: false,
            running: 0,
        };
        for container in &plan.containers {
            let Some(info) = self.engine.inspect_container(&container.name).await? else {
                drift.drifted = true;
                continue;
            };
            if info.state.and_then(|state| state.running) == Some(true) {
                drift.running += 1;
            }
            let label = info
                .config
                .and_then(|config| config.labels)
                .and_then(|labels| labels.get(LABEL_RENDER).cloned());
            if label.as_deref() != Some(digest(container).as_str()) {
                drift.drifted = true;
            }
        }
        Ok(drift)
    }
}
