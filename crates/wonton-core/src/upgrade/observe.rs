//! `observing` 阶段判断服务是不是还好好的。

use std::collections::HashMap;

use super::host::ServiceContainer;
use crate::l10n;
use crate::plugin::languages::Languages;

/// 刚进入观察时各个长期服务的样子：之后换了容器或者重启次数变了，就是崩过。
#[derive(Debug, Default)]
pub struct Baseline(HashMap<String, (String, i64, String)>);

impl Baseline {
    pub fn of(containers: &[ServiceContainer]) -> Self {
        let long_running = containers.iter().filter(|c| c.long_running);
        Self(
            long_running
                .map(|c| {
                    (
                        c.service.clone(),
                        (c.id.clone(), c.restart_count, c.image.clone()),
                    )
                })
                .collect(),
        )
    }

    /// 第一个有问题的服务和问题；都好时是 `None`。设了重启策略的长期服务退出、重启、变成不健康或者换了镜像都算；一次性
    /// 服务（如 `hydra-migrate`）只要求退出码为 0。
    pub fn problem(&self, containers: &[ServiceContainer], langs: &Languages) -> Option<String> {
        let text = |id: &str, args: &[(&str, &str)]| {
            l10n::text(
                langs,
                id,
                &args
                    .iter()
                    .map(|(name, value)| (*name, (*value).into()))
                    .collect::<Vec<_>>(),
            )
        };
        for (service, (id, restarts, image)) in &self.0 {
            let Some(now) = containers.iter().find(|c| &c.service == service) else {
                return Some(text(
                    "upgrade-service-container-gone",
                    &[("service", service)],
                ));
            };
            if &now.id != id {
                return Some(text(
                    "upgrade-service-container-replaced",
                    &[("service", service)],
                ));
            }
            if &now.image != image {
                return Some(text(
                    "upgrade-service-image-changed",
                    &[("service", service), ("image", &now.image)],
                ));
            }
            if now.restart_count != *restarts {
                return Some(text("upgrade-service-restarted", &[("service", service)]));
            }
            if now.status != "running" {
                return Some(text(
                    "upgrade-service-status",
                    &[("service", service), ("status", &now.status)],
                ));
            }
            if now.health.as_deref() == Some("unhealthy") {
                return Some(text("upgrade-service-unhealthy", &[("service", service)]));
            }
        }
        containers
            .iter()
            .filter(|c| !c.long_running && c.status == "exited" && c.exit_code != 0)
            .map(|c| {
                text(
                    "upgrade-one-shot-exit-code",
                    &[("service", &c.service), ("code", &c.exit_code.to_string())],
                )
            })
            .next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs() -> Languages {
        Languages::from_header(None)
    }

    fn service(name: &str) -> ServiceContainer {
        ServiceContainer {
            id: format!("id-{name}"),
            service: name.into(),
            image: format!("{name}:1"),
            status: "running".into(),
            long_running: true,
            ..Default::default()
        }
    }

    fn fails_after(change: impl FnOnce(&mut ServiceContainer)) -> bool {
        let before = vec![service("core")];
        let mut after = before.clone();
        change(&mut after[0]);
        Baseline::of(&before).problem(&after, &langs()).is_some()
    }

    #[test]
    fn a_long_running_service_must_stay_as_it_was() {
        assert!(!fails_after(|_| {}));
        assert!(fails_after(|c| c.status = "exited".into()));
        assert!(fails_after(|c| c.restart_count = 1));
        assert!(fails_after(|c| c.health = Some("unhealthy".into())));
        assert!(fails_after(|c| c.image = "core:2".into()));
        assert!(fails_after(|c| c.id = "other".into()));
    }

    /// `hydra-migrate` 跑完退出是正常的：只有退出码不为 0 才算失败。
    #[test]
    fn a_one_shot_service_only_has_to_exit_cleanly() {
        let one_shot = ServiceContainer {
            long_running: false,
            status: "exited".into(),
            ..service("hydra-migrate")
        };
        let baseline = Baseline::of(std::slice::from_ref(&one_shot));
        assert_eq!(
            baseline.problem(std::slice::from_ref(&one_shot), &langs()),
            None
        );
        let failed = ServiceContainer {
            exit_code: 1,
            ..one_shot
        };
        assert!(baseline.problem(&[failed], &langs()).is_some());
    }
}
