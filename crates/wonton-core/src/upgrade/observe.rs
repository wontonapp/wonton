//! `observing` 阶段判断服务是不是还好好的。

use std::collections::HashMap;

use super::host::ServiceContainer;

/// 刚进入观察时各个长期服务的样子：之后换了容器或者重启次数变了，就是崩过。
#[derive(Debug, Default)]
pub struct Baseline(HashMap<String, (String, i64, String)>);

impl Baseline {
    pub fn of(containers: &[ServiceContainer]) -> Self {
        let long_running = containers.iter().filter(|c| c.long_running);
        Self(
            long_running
                .map(|c| {
                    let seen = (c.id.clone(), c.restart_count, c.image.clone());
                    (c.service.clone(), seen)
                })
                .collect(),
        )
    }

    /// 第一个有问题的服务和问题；都好时是 `None`。设了重启策略的长期服务退出、重启、变成不健康或者换了镜像都算；一次性
    /// 服务（如 `hydra-migrate`）只要求退出码为 0。
    pub fn problem(&self, containers: &[ServiceContainer]) -> Option<String> {
        for (service, (id, restarts, image)) in &self.0 {
            let Some(now) = containers.iter().find(|c| &c.service == service) else {
                return Some(format!("服务 {service} 的容器不见了"));
            };
            if &now.id != id {
                return Some(format!("服务 {service} 的容器被换掉了"));
            }
            if &now.image != image {
                return Some(format!("服务 {service} 换了镜像：{}", now.image));
            }
            if now.restart_count != *restarts {
                return Some(format!("服务 {service} 重启了"));
            }
            if now.status != "running" {
                return Some(format!("服务 {service} 的状态是 {}", now.status));
            }
            if now.health.as_deref() == Some("unhealthy") {
                return Some(format!("服务 {service} 变成不健康"));
            }
        }
        containers
            .iter()
            .filter(|c| !c.long_running && c.status == "exited" && c.exit_code != 0)
            .map(|c| format!("一次性服务 {} 的退出码是 {}", c.service, c.exit_code))
            .next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        Baseline::of(&before).problem(&after).is_some()
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
        assert_eq!(baseline.problem(std::slice::from_ref(&one_shot)), None);
        let failed = ServiceContainer {
            exit_code: 1,
            ..one_shot
        };
        assert!(baseline.problem(&[failed]).is_some());
    }
}
