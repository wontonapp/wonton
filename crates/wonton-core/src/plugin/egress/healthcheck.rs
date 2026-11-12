//! 配了代理的插件的健康检查不走代理（ADR-0023 的“健康检查不走代理”）。
//!
//! Docker 的健康检查和容器共用环境变量，没有单独的设置；busybox wget 不看 `no_proxy`，检查 `127.0.0.1` 也发给代理，
//! 插件就一直不健康。插件作者不该为此操心，所以渲染时改写。其他程序（`curl`、`python`、`/magpie`）的回环地址都不走
//! 代理（实测），不改。

use bollard::models::{ContainerCreateBody, HealthConfig};

use crate::plugin::manager::{InstallError, PluginManager};
use crate::plugin::render::{Deployment, InstallContext};

const UNSET: &str = "unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY; ";
const SHELLS: [&str; 4] = ["sh", "ash", "bash", "dash"];

/// 程序名按路径的最后一段判断（`/usr/bin/wget` 算 `wget`）。
fn program(arg: &str) -> &str {
    arg.rsplit('/').next().unwrap_or(arg)
}

/// `-c` 的脚本在第几个参数。选项可以连写（`sh -ec`）。
fn script_index(args: &[String]) -> Option<usize> {
    let flag = args.iter().position(|arg| {
        arg.len() > 1 && arg.starts_with('-') && !arg.starts_with("--") && arg.ends_with('c')
    })?;
    (flag + 1 < args.len()).then_some(flag + 1)
}

/// 改写后的 `test`；不用改时返回 `None`。
pub fn rewrite(test: &[String]) -> Option<Vec<String>> {
    let (kind, rest) = test.split_first()?;
    let mut out = test.to_vec();
    match kind.as_str() {
        "CMD-SHELL" => {
            let script = rest.first()?;
            out[1] = format!("{UNSET}{script}");
            Some(out)
        }
        "CMD" => {
            // `busybox wget`、`busybox sh`：真正的程序在 busybox 后面。
            let at = usize::from(program(rest.first()?) == "busybox");
            let name = program(rest.get(at)?);
            // `out` 比 `rest` 多一个开头的 `CMD`。
            let at = at + 1;
            if name == "wget" {
                out.splice(at + 1..at + 1, ["-Y".to_owned(), "off".to_owned()]);
                return Some(out);
            }
            if SHELLS.contains(&name) {
                let script = at + 1 + script_index(&out[at + 1..])?;
                out[script] = format!("{UNSET}{}", out[script]);
                return Some(out);
            }
            None
        }
        _ => None,
    }
}

/// 改写容器的健康检查。compose 没写 `test` 时，要用 [`apply_image`] 把镜像自带的带上。
pub fn apply(body: &mut ContainerCreateBody) {
    let Some(check) = &mut body.healthcheck else {
        return;
    };
    if let Some(rewritten) = check.test.as_deref().and_then(rewrite) {
        check.test = Some(rewritten);
    }
}

/// compose 没写 `test` 时，改写镜像自带的 `HEALTHCHECK`：要改才写进容器，时间参数原样带上，compose 写了的以 compose
/// 为准。`NONE` 不改。
pub fn apply_image(body: &mut ContainerCreateBody, image: &HealthConfig) {
    if body
        .healthcheck
        .as_ref()
        .is_some_and(|check| check.test.as_ref().is_some_and(|t| !t.is_empty()))
    {
        return;
    }
    let Some(test) = image.test.as_deref().and_then(rewrite) else {
        return;
    };
    let check = body.healthcheck.get_or_insert_with(Default::default);
    check.test = Some(test);
    check.interval = check.interval.or(image.interval);
    check.timeout = check.timeout.or(image.timeout);
    check.start_period = check.start_period.or(image.start_period);
    check.start_interval = check.start_interval.or(image.start_interval);
    check.retries = check.retries.or(image.retries);
}

impl PluginManager {
    /// 配了代理时，把 compose 没写 `test` 的容器的镜像自带健康检查也改写。镜像要先拉下来。
    pub(in crate::plugin) async fn rewrite_image_healthchecks(
        &self,
        ctx: &InstallContext,
        plan: &mut Deployment,
    ) -> Result<(), InstallError> {
        if ctx.egress.is_none() {
            return Ok(());
        }
        for container in &mut plan.containers {
            let isolated = container
                .body
                .host_config
                .as_ref()
                .and_then(|host| host.network_mode.as_deref())
                == Some("none");
            if isolated {
                continue;
            }
            if let Some(check) = self.engine.image_healthcheck(&container.image).await? {
                apply_image(&mut container.body, &check);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn rewritten(words: &[&str]) -> Option<Vec<String>> {
        rewrite(&argv(words))
    }

    #[test]
    fn shell_forms_clear_the_proxy_variables_first() {
        assert_eq!(
            rewritten(&["CMD-SHELL", "wget -q -O /dev/null http://127.0.0.1/"]).unwrap(),
            [
                "CMD-SHELL",
                "unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY; wget -q -O /dev/null http://127.0.0.1/"
            ]
        );
        for shell in ["sh", "ash", "bash", "dash", "/bin/sh", "/usr/bin/bash"] {
            assert_eq!(
                rewritten(&["CMD", shell, "-c", "wget http://127.0.0.1/"]).unwrap(),
                [
                    "CMD",
                    shell,
                    "-c",
                    "unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY; wget http://127.0.0.1/"
                ]
            );
        }
        assert_eq!(
            rewritten(&["CMD", "sh", "-ec", "true"]).unwrap()[3],
            "unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY; true"
        );
        assert_eq!(
            rewritten(&["CMD", "busybox", "sh", "-c", "true"]).unwrap()[4],
            "unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY; true"
        );
    }

    #[test]
    fn wget_gets_its_proxy_switched_off() {
        assert_eq!(
            rewritten(&["CMD", "wget", "-q", "http://127.0.0.1/"]).unwrap(),
            ["CMD", "wget", "-Y", "off", "-q", "http://127.0.0.1/"]
        );
        assert_eq!(
            rewritten(&["CMD", "/usr/bin/wget", "http://127.0.0.1/"]).unwrap(),
            ["CMD", "/usr/bin/wget", "-Y", "off", "http://127.0.0.1/"]
        );
        assert_eq!(
            rewritten(&["CMD", "busybox", "wget", "http://127.0.0.1/"]).unwrap(),
            ["CMD", "busybox", "wget", "-Y", "off", "http://127.0.0.1/"]
        );
        assert_eq!(
            rewritten(&["CMD", "/bin/busybox", "wget", "http://127.0.0.1/"]).unwrap(),
            [
                "CMD",
                "/bin/busybox",
                "wget",
                "-Y",
                "off",
                "http://127.0.0.1/"
            ]
        );
    }

    #[test]
    fn other_programs_and_none_stay_as_they_are() {
        for words in [
            &["CMD", "curl", "-f", "http://127.0.0.1/"][..],
            &["CMD", "python", "-c", "import urllib"],
            &["CMD", "/magpie", "healthcheck"],
            &["CMD", "busybox", "nc", "-z", "127.0.0.1", "80"],
            &["CMD", "sh", "healthcheck.sh"],
            &["NONE"],
            &[],
        ] {
            assert_eq!(rewritten(words), None, "{words:?}");
        }
    }

    fn image_check() -> HealthConfig {
        HealthConfig {
            test: Some(argv(&["CMD-SHELL", "wget http://127.0.0.1/"])),
            interval: Some(1),
            timeout: Some(2),
            start_period: Some(3),
            start_interval: Some(4),
            retries: Some(5),
        }
    }

    #[test]
    fn the_image_healthcheck_is_rewritten_when_compose_has_no_test() {
        let mut body = ContainerCreateBody::default();
        apply_image(&mut body, &image_check());
        let check = body.healthcheck.unwrap();
        assert!(check.test.unwrap()[1].starts_with("unset "));
        assert_eq!(
            (check.interval, check.timeout, check.start_period),
            (Some(1), Some(2), Some(3))
        );
        assert_eq!((check.start_interval, check.retries), (Some(4), Some(5)));

        // compose 只写了时间参数：test 从镜像取，compose 写了的时间参数不变。
        let mut body = ContainerCreateBody {
            healthcheck: Some(HealthConfig {
                interval: Some(100),
                ..Default::default()
            }),
            ..Default::default()
        };
        apply_image(&mut body, &image_check());
        let check = body.healthcheck.unwrap();
        assert_eq!(check.interval, Some(100));
        assert_eq!(check.timeout, Some(2));
        assert!(check.test.unwrap()[1].starts_with("unset "));
    }

    #[test]
    fn the_image_healthcheck_is_ignored_when_compose_wrote_a_test_or_nothing_needs_changing() {
        let own = HealthConfig {
            test: Some(argv(&["CMD", "curl", "-f", "http://127.0.0.1/"])),
            ..Default::default()
        };
        let mut body = ContainerCreateBody {
            healthcheck: Some(own.clone()),
            ..Default::default()
        };
        apply_image(&mut body, &image_check());
        assert_eq!(body.healthcheck, Some(own));

        let disabled = HealthConfig {
            test: Some(argv(&["NONE"])),
            ..Default::default()
        };
        let mut body = ContainerCreateBody {
            healthcheck: Some(disabled.clone()),
            ..Default::default()
        };
        apply_image(&mut body, &image_check());
        assert_eq!(
            body.healthcheck,
            Some(disabled.clone()),
            "compose 关了健康检查"
        );

        let mut body = ContainerCreateBody::default();
        apply_image(&mut body, &disabled);
        assert_eq!(body.healthcheck, None, "镜像的 NONE 不改");
    }
}
