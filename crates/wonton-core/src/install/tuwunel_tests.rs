//! 全新安装的 Tuwunel 起得来：`init` 写出的配置，加上正式 compose 里给 tuwunel 的命令、环境变量和挂载，起真的 Tuwunel。
//! 配置里引用的文件没挂进容器时它当场退出（#605 漏挂 TLS 证书，全新安装就卡在这一步）。

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::test_docker::Names;

const WAIT: Duration = Duration::from_secs(90);

/// 只读 tuwunel：别的服务里有长格式的挂载（核心的 /dev），按这里的类型读不了。
#[derive(Deserialize)]
struct Compose {
    services: Services,
}

#[derive(Deserialize)]
struct Services {
    tuwunel: Service,
}

#[derive(Deserialize)]
struct Service {
    image: String,
    #[serde(default)]
    command: Vec<String>,
    /// `<<` 合并键读出来可能是映射，只带值是字符串的。
    #[serde(default)]
    environment: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    volumes: Vec<String>,
}

#[test]
#[ignore = "需要 Docker，跑一遍约半分钟"]
fn a_fresh_install_starts_tuwunel_with_the_compose_mounts() {
    let dir = tempfile::tempdir().unwrap();
    super::init(dir.path(), None).unwrap();
    let compose: Compose = serde_saphyr::from_str(super::COMPOSE).unwrap();
    let tuwunel = compose.services.tuwunel;

    // 守卫丢弃时（panic 也算）删掉登记的容器和它的匿名卷。
    let mut names = Names::new();
    let name = names.oneshot_container("tuwunel");
    let mut args = vec![
        "run".to_owned(),
        "--detach".to_owned(),
        "--name".to_owned(),
        name.clone(),
    ];
    args.extend(bind_mounts(&tuwunel.volumes, dir.path()));
    // 本机 Docker 客户端配了代理时会注入容器；compose 里也是全部置空。
    for var in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
        args.extend(["--env".to_owned(), format!("{var}=")]);
    }
    // 健康检查（`tuwunel --health-check`）不带 -c，靠 TUWUNEL_CONFIG 找配置。
    for (var, value) in &tuwunel.environment {
        if let Some(value) = value.as_str() {
            args.extend(["--env".to_owned(), format!("{var}={value}")]);
        }
    }
    // 测试机按 docker-test-images.txt 预先放好的是不带 digest 的标签。
    args.push(tuwunel.image.split('@').next().unwrap().to_owned());
    args.extend(tuwunel.command);
    let started = docker(&args);
    assert!(
        started.status.success(),
        "起 Tuwunel：{}",
        String::from_utf8_lossy(&started.stderr)
    );

    let deadline = Instant::now() + WAIT;
    loop {
        if docker(&["exec", &name, "tuwunel", "--health-check"])
            .status
            .success()
        {
            return;
        }
        let running = docker(&["inspect", "--format", "{{.State.Running}}", &name]);
        let gone = String::from_utf8_lossy(&running.stdout).trim() != "true";
        if gone || Instant::now() > deadline {
            let logs = docker(&["logs", "--tail", "30", &name]);
            panic!(
                "Tuwunel 没有起来（{}）：\n{}{}",
                if gone { "已退出" } else { "超时" },
                String::from_utf8_lossy(&logs.stdout),
                String::from_utf8_lossy(&logs.stderr)
            );
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// compose 里的绑定挂载换成安装目录下的真实路径；命名卷（数据库）不挂，容器里用临时的。
fn bind_mounts(volumes: &[String], install_dir: &Path) -> Vec<String> {
    let install_dir = install_dir.to_string_lossy();
    volumes
        .iter()
        .filter(|volume| volume.starts_with("${WONTON_INSTALL_DIR}/"))
        .flat_map(|volume| {
            let mount = volume.replace("${WONTON_INSTALL_DIR}", &install_dir);
            ["--volume".to_owned(), mount]
        })
        .collect()
}

fn docker<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Output {
    Command::new("docker")
        .args(args)
        .output()
        .expect("运行 Docker")
}
