//! 真实 Docker 测试共用的名字和清理：CI 的几个 runner 共用一台 Docker，测试的资源名一律带每次运行唯一的
//! 实例名（进程号加随机数），结束后按实例标签和登记的名字回收——panic 也回收，只删自己登记和实例名下的，
//! 不按前缀清别人的资源。
//!
//! 单元测试经 `crate::test_docker` 用它；`tests/` 里的集成测试经 `#[path]` 把本文件编进自己的测试
//! crate，所以这里只引用 `wonton_core` 的公开接口。

use wonton_core::plugin::compose::PullPolicy;
use wonton_core::plugin::docker::Engine;
use wonton_core::plugin::names::{Instance, LABEL_INSTANCE};

/// 拉测试镜像，失败时挂上“基础设施失败”和镜像引用：和断言失败分开，一眼看得出是拉不到而不是行为不对。
/// 返回实际能用的引用（可能是加速站），建容器用它，和预拉用的是同一个。
#[allow(dead_code)]
pub async fn ensure_image(engine: &Engine, image: &str) -> String {
    ensure_image_through(engine, image, &[]).await
}

/// 同 [`ensure_image`]，带加速站列表。
#[allow(dead_code)]
pub async fn ensure_image_through(engine: &Engine, image: &str, accelerated: &[String]) -> String {
    match engine
        .ensure_image(image, PullPolicy::Missing, accelerated)
        .await
    {
        Ok(resolved) => resolved,
        Err(error) => panic!("基础设施失败：拉不到镜像 {image}：{error}"),
    }
}

/// 每次运行唯一的实例名：进程号加随机数。同一进程里并行的测试靠随机数区分，不同进程靠进程号；进程号
/// 复用（上次运行被杀留下的资源）时还有随机数兜底。
// 本文件编进多个测试目标（wonton-core 的单元测试和 tests/ 的集成测试），每个目标只用其中一部分。
#[allow(dead_code)]
pub fn test_instance() -> Instance {
    let name = format!("wt{:x}{:x}", std::process::id(), getrandom::u32().unwrap());
    Instance::parse(Some(&name)).unwrap()
}

/// 一套测试的资源登记表。`Instance` 派生的资源名（插件容器、网络、命名卷）不用登记：核心给它们打了
/// 实例标签，守卫按标签统一清；不经过 `Instance` 的一次性容器和网络（tusd、扮演核心的容器、探针网络）
/// 用 [`Names::oneshot_container`] 和 [`Names::oneshot_network`] 登记。
///
/// 守卫丢弃时（panic 展开也算）先删容器再删网络最后删卷；单删失败不挡其余的，留错的用 `docker ps -a`
/// 按唯一实例名能找到。清理走 docker CLI：Drop 是同步的，Engine 的异步接口用不上，Tusd、升级器的
/// `Install` 等既有守卫也这么写。
pub struct Names {
    instance: Instance,
    containers: Vec<String>,
    networks: Vec<String>,
}

// 本文件编进多个测试目标（wonton-core 的单元测试和 tests/ 的集成测试），每个目标只用其中一部分。
#[allow(dead_code)]
impl Names {
    pub fn new() -> Self {
        Self {
            instance: test_instance(),
            containers: Vec::new(),
            networks: Vec::new(),
        }
    }

    /// 本套测试资源的实例。
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    /// 实例名的字符串形式，给收 `&str` 的入口（如插件测试的 `manager_in_instance`）。
    pub fn name(&self) -> &str {
        self.instance.name().unwrap_or_default()
    }

    /// 插件容器 `<插件网络>--<服务名>`。
    pub fn container(&self, slug: &str, service: &str) -> String {
        self.instance.container(slug, service)
    }

    /// 插件网络 `wonton-<实例名>--<slug>`。
    pub fn network(&self, slug: &str) -> String {
        self.instance.network(slug)
    }

    /// 登记并返回一个一次性容器的名字 `wonton-<实例名>-<用途>`。
    pub fn oneshot_container(&mut self, what: &str) -> String {
        let name = self.oneshot_name(what);
        self.containers.push(name.clone());
        name
    }

    /// 登记一个名字已定的容器：出口容器的名字和它自己的网络名是同一个（`Relay::name`）。
    pub fn register_container(&mut self, name: String) -> String {
        self.containers.push(name.clone());
        name
    }

    /// 登记一个名字已定的网络：测试特意造成"同名的网络不是本插件的"（不带实例标签）时用。
    pub fn register_network(&mut self, name: String) -> String {
        self.networks.push(name.clone());
        name
    }

    /// 登记并返回一个一次性网络的名字。
    pub fn oneshot_network(&mut self, what: &str) -> String {
        let name = self.oneshot_name(what);
        self.networks.push(name.clone());
        name
    }

    /// 一个一次性镜像的标签 `wonton-<实例名>-<用途>`。守卫只清容器、网络和卷：镜像由测试自己删
    /// （标签可能被移到别的镜像上，按标签强删会伤到那个镜像）。
    pub fn oneshot_image(&mut self, what: &str) -> String {
        self.oneshot_name(what)
    }

    fn oneshot_name(&self, what: &str) -> String {
        format!("wonton-{}-{what}", self.name())
    }
}

impl Default for Names {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Names {
    fn drop(&mut self) {
        let filter = format!("label={LABEL_INSTANCE}={}", self.name());
        let mut containers = std::mem::take(&mut self.containers);
        containers.extend(cli_lines(["ps", "-aq", "--filter", &filter]));
        for name in &containers {
            cli(&["rm", "--force", "--volumes", name]);
        }
        let mut networks = std::mem::take(&mut self.networks);
        networks.extend(cli_lines([
            "network",
            "ls",
            "--filter",
            &filter,
            "--format",
            "{{.Name}}",
        ]));
        for name in &networks {
            cli(&["network", "rm", name]);
        }
        for name in cli_lines(["volume", "ls", "--filter", &filter, "--format", "{{.Name}}"]) {
            cli(&["volume", "rm", &name]);
        }
    }
}

fn cli(args: &[&str]) {
    let _ = std::process::Command::new("docker").args(args).output();
}

/// docker CLI 输出里的非空行；CLI 不在或守护进程不可达时为空，留给下一个测试或人去收。
fn cli_lines<const N: usize>(args: [&str; N]) -> Vec<String> {
    let output = std::process::Command::new("docker").args(args).output();
    String::from_utf8_lossy(&output.map(|o| o.stdout).unwrap_or_default())
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}
