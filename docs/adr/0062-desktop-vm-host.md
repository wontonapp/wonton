# ADR-0062 Mac 和 Windows 当服务器：云吞自带一台隐藏的 Linux 虚拟机

- 状态：已接受（2026-11-08，方向由主人在卡片上定）。部分取代 [ADR-0002](0002-linux-docker-only.md) 的“不支持 macOS 或 Windows 作为宿主”，
  ADR-0002 的其余决定（服务端是 Linux + Docker、核心是容器、不做整机 A/B 升级）不变

## 背景

有 Linux 主机的用户是少数，多数人家里常开的是一台 Mac 或 Windows 电脑。ADR-0002 只支持 Linux + Docker，这些人装不上。

在 Mac、Windows 上跑云吞，现成的路有三条：让用户自己装 Docker Desktop 或 OrbStack；把核心改成原生的 macOS、Windows 程序；
云吞自己带一台 Linux 虚拟机。前两条的问题：

- Docker Desktop、OrbStack 要用户自己装、要登录桌面才启动，商用要许可证；核心在它们的虚拟机里看到的是虚拟机的 NAT 网卡，
  会把这个手机连不到的地址告诉手机（实测见下）。
- 原生核心要放弃 Docker 管插件这件事，插件全部重写或另找运行时，和 ADR-0006 冲突。

## 决策

1. **服务端照旧是 Linux + Docker。** 在 Mac、Windows 上，云吞自带并管理一台 Linux 虚拟机，虚拟机里装 Docker 和核心，
   和装在 Linux 主机上的一样。核心不分辨自己在不在这台虚拟机里，只认启动器经环境变量声明的事实（第 3 条）。
2. **虚拟机运行时用现成的。** Mac 用 [Lima](https://lima-vm.io/)（Apache-2.0）的 vz 驱动（Apple 的 Virtualization.framework），
   装在用户目录、不用 brew。Windows 按版本分两条（实测见下）：
   - 专业版、企业版用 Hyper-V，虚拟机接绑在宿主有线网卡上的外部交换机。来宾自己有局域网地址，不用宣告地址，也不用转发。
   - 家庭版没有 Hyper-V，用 WSL2 的一个 `wonton` 发行版。宿主上的辅助程序转发 TCP 8480 和 UDP 8482，宣告地址写宿主的局域网地址。
     WSL 的 mirrored 网络能省掉转发，但只能在用户全局的 `.wslconfig` 里开，会改掉用户别的发行版，所以不用。
3. **核心分开“端口绑在哪”和“告诉手机的地址”。** 新增 `WONTON_ANNOUNCE_ADDRESSES`（逗号分隔的私有 IP，可带前缀，
   如 `192.168.1.20/24`），由启动器写宿主的局域网地址：
   - 设了它：隧道、配对码、插件的局域网提示和出网直连网段（ADR-0023）都用它；插件端口在虚拟机里绑 `0.0.0.0`，
     由虚拟机运行时的端口转发开到宿主的局域网上。
   - 没设时行为不变。
   - 核心认出自己跑在 Docker Desktop、OrbStack、WSL2 这类桌面虚拟机里（内核串含 `linuxkit`、`orbstack`、
     `microsoft-standard-WSL2`），又没设宣告地址时，如实报错 `LanFailure::DesktopVm`，不静默地把虚拟机地址告诉手机。
     只按内核串认，不按网卡类型或网段猜：Proxmox、ESXi、Unraid 里桥接的虚拟网卡是正常的局域网地址。
4. **网络先用 NAT 加端口转发**（Lima 的 usernet 和 hostagent）：Lima 把宿主的 TCP 8480、UDP 8482 转进虚拟机，
   手机在局域网里直连（实测见下）。桥接（socket_vmnet，要 root）让虚拟机有自己的局域网地址、不用宣告地址和转发，
   P0 还没测，测完再定是否改用。
5. **模型在宿主上原生跑。** 虚拟机里没有 Apple 的 GPU，模型放虚拟机里只能用 CPU。用户装官方的 Ollama.app（或 LM Studio），
   magpie 加一个“这台电脑上的 OpenAI 兼容端点”，地址由启动器给（Lima 里是 `host.lima.internal`）。云吞不管宿主上 Ollama 的
   生命周期，插件规范不加“宿主原生服务”。
6. **虚拟机的系统盘是可丢弃的。** 升级时整盘替换（不在里面 `apt upgrade`），数据放单独的数据盘，升级和回滚仍按摘要走
   （ADR-0025）。这就是我们分发并维护的一份系统镜像，也是唯一的一份：AGENTS.md 的产品边界写的是“只在 Mac、Windows 上
   带一台不可变的 Linux 虚拟机，系统盘整盘替换”。
7. **宿主侧的辅助程序只做三件事**：管虚拟机的生命周期、阻止宿主睡眠、维护端口转发和宣告地址（宿主换了局域网地址时更新）。
   首次配对照旧用配对码，不给“本机 App 自动成为主人”开特例。
   辅助程序照“厚 Rust 核心、薄原生壳”写：逻辑（什么时候起停虚拟机、宣告哪个地址、转发什么）都在 Rust 里；
   Swift、PowerShell 只做平台调用：LaunchDaemon、计划任务、Hyper-V 和 WSL 的命令。

## 理由

- 插件、升级、隔离全部沿用 Linux 上的那一套，核心只多一个环境变量和一种失败类别。
- Lima 自带 TCP 和 UDP 的端口转发（`proto: any`、`hostIP: 0.0.0.0`），不用自己写转发器。
- 局域网直连比中继快两个数量级（实测 1–2 ms 对 800 ms），桌面虚拟机必须把宿主的局域网地址告诉手机。

## 代价与风险

- **我们开始维护一份客户系统**：Debian 云镜像、Docker Engine、来宾内核的安全更新都归我们发。用整盘替换控制工作量，
  但发布流程要多一条“虚拟机镜像”的线。
- **内存**：vz 不把来宾的页缓存还给宿主（实测见下），4 GiB 的虚拟机在宿主上常驻接近 4 GiB。8 GB 内存的 Mac 跑不动
  核心加 Immich；安装时低于 16 GB 要明说。
- **FileVault 和重启**：Mac 默认开 FileVault，重启后在输入密码前数据卷不解锁，虚拟机起不来。开着 FileVault 时 App 和手机要如实说
  “电脑重启后要输一次密码”；“断电后自动开机”只引导，不替用户改。
- **睡眠**：宿主睡着时核心、Tuwunel、隧道都停了，手机只能说“连不上家里的服务器，最后在线于 X”，没有云端在线服务时不伪装成
  “Mac 睡着了”。辅助程序在接电源时阻止系统睡眠。
- **签名和分发**：limactl、hostagent 要用我们的 Developer ID 重签并带虚拟化授权；带 LaunchDaemon 的服务端包走不了
  Mac App Store，只能 Developer ID 的 pkg 或 dmg。
- **国内首装**：Debian 云镜像和容器镜像都从境外拉，没有代理时很慢；首装要带离线的系统盘和核心镜像，或者走镜像加速。
- **桌面文件夹**：virtiofs、9P 不可靠地传 inotify，ADR-0020 的属主、硬链接要求在这类文件系统上不成立；P0 不共享宿主文件夹。

## 要主人定的

- **Apple Developer ID**：按年收费，limactl 和 hostagent 重签、pkg 公证都要用。主人说过“先实测再说”，P0 的数据见下。
- **国内首装的镜像来源**：用公共镜像站（南京大学、DaoCloud 这类第三方，实测见下），还是只带离线的系统盘和镜像包。
  用镜像站要信任第三方，只带离线包则安装包大、版本跟得慢。
- **Windows 家庭版 WSL 的内存上限**：只能在用户全局的 `.wslconfig` 里改，默认是宿主内存的一半。我们不改它；
  要不要在 App 里引导用户自己改，要主人定。

## 待验证

- Lima 桥接（socket_vmnet）对比 NAT 加转发：socket_vmnet 要 root，等主人在那台 Mac 上装好再测。
- 睡眠唤醒后来宾的时钟和 OIDC 登录；重启后的行为（FileVault 开和关）。那台 Mac 开着 FileVault，重启后要有人在机器前输密码，
  P0 不在那台 Mac 上重启。
- 加上 Immich 后的内存。
- 真手机在主人家里的局域网里连 Mac 上的虚拟机（宣告宿主地址）和 Windows 上的虚拟机：待主人在家补测。P0 用 Android
  模拟器和同一交换机上的另一台虚拟机代替手机。
- Windows：和已有的 Docker Desktop 能否并存；Wi-Fi 网卡上的外部交换机（P0 只测了有线）；系统更新后的重启；关掉 Windows 重启后自动登录时，开机任务照样起来。
- 客户端核心编到 Windows（`cargo check` 两个 Windows 目标）。

## 验证结果（P0）

### Lima vz 虚拟机（2026-11-07）

条件：一台 Mac（M4，16 GB，macOS 27.0，FileVault 开），Lima 2.2.1（vz，usernet），Debian 13 云镜像
（6.12.115 arm64），4 核、4 GiB、20 GiB 磁盘，不挂宿主目录；Docker 29.9.3；云吞 v0.1.0-beta.4（beta 渠道）。
外网经主人 Mac 上的代理（那台 Mac 直连 GitHub 只有几十 KB/s）。

- 建虚拟机到可用：589 秒，其中下载 Debian 云镜像（一次失败后改拉 daily 镜像）和装 Docker 占大半。
- 装云吞：从执行安装脚本到核心、Hydra、Tuwunel 都起来用了 48 秒（核心镜像在前一次运行里已经拉好），magpie 再过 32 秒装好。
- 磁盘：虚拟机磁盘实际占用 3.4 GB（基础套件），Lima 的镜像缓存 1.4 GB。
- 内存：来宾里基础套件（核心、Hydra、Tuwunel、tusd、出口、magpie）容器合计约 180 MB，来宾已用 835 MB；宿主上 vz 进程常驻
  3.75 GB，来宾的页缓存不还给宿主。
- 来宾里读到的：`/proc/version` 是普通的 Debian 内核串；DMI 是 `Apple Virtualization Generic Platform` / `Apple Inc.`。
  后者说明“在 Apple 虚拟化里”，不说明网络是 NAT（vz 桥接的虚拟机也是这个值），所以 Lima 不靠检测，由启动器声明。
- 核心宣告的地址：安装终端打出的配对码里 `addr=192.0.2.20:8482`，是虚拟机的 NAT 地址。
- 端口转发：`portForwards` 写 8480/tcp 和 8482/udp 的 `hostIP: 0.0.0.0` 后，宿主上 limactl 监听 `*:8480`、`*:8482`，
  局域网里 `http://192.0.2.15:8480/` 回 200。

### 手机经局域网直连（2026-11-07）

条件：同上的虚拟机；Android 模拟器（API 35 x86_64，在同一局域网的另一台主机上，不走代理），App 为 e5a69c37 的 Debug 构建（比核心新一天）。
配对码由来宾里的 `wonton-core pair <地址>` 生成。

| 配对码里的地址 | 路径 | 延迟 | 丢包 | 说明 |
| --- | --- | --- | --- | --- |
| `192.0.2.15:8482`（宿主的局域网地址） | 局域网 | 1–2 ms | 0 / 1596 | 观察 5 分钟，断开 0 次 |
| `192.0.2.20:8482`（核心现在宣告的） | 中继 | 797 ms | 1 / 267 | 直连超时后落到 n0 中继 |

UDP 经 Lima hostagent 转发的直连稳定，宣告宿主的局域网地址就够；宣告虚拟机地址时只能走中继。

### 模型速度

条件：同一台 Mac，Ollama 0.40.15，`llama3.2:3b`，温度 0，每次生成 256 个 token，预热一次后测 3 次。

- 宿主原生（Metal GPU）：生成 45.0–45.2 tokens/s。
- 虚拟机里（4 核 CPU，Ollama 官方容器 `ollama/ollama:0.40.15`）：生成 32.3–32.6 tokens/s，约为原生的 72%；
  模型加载后容器占 2.5 GiB，4 GiB 的虚拟机里剩不下多少给云吞和插件。

3B 模型上速度差得不多，决定放宿主上主要因为内存：模型放进虚拟机，虚拟机就得按模型的大小分内存，而 vz 不还页缓存；
更大的模型在 CPU 上的差距也会变大（没测）。

### Windows：Hyper-V 外部交换机（2026-11-07）

条件：Windows 11 专业版（26300），16 GB，有线 2.5GbE；外部交换机绑这块网卡。Debian 13 云镜像（amd64）
的 raw 盘加 VHD 盘尾后转成 VHDX，cloud-init 用 NoCloud 种子盘（卷标 `cidata` 的 FAT32 VHDX）；第 2 代虚拟机，4 核、
4 GiB 固定内存（后来为了和 WSL 并存改成 2 GiB，容器 55 秒内全部恢复），安全启动用 Microsoft UEFI CA 模板。另一台 1 核、1 GiB 的同款虚拟机接同一个交换机，当局域网里的手机。

- 从开机到 SSH 可登录：约 4 分钟（含 cloud-init 首次启动）。cloud-init 的 `runcmd` 在网络就绪前就执行了，
  `curl get.docker.com` 拿到空内容；启动器要等 `network-online` 再装。
- 装 Docker（阿里云镜像）：80 秒。
- 拉镜像：这台机器连不上 Docker Hub，GHCR 拉到 131 MB 后不动了。改从镜像站按摘要拉（GHCR 用南京大学的，
  Docker Hub 用 DaoCloud；DaoCloud 有白名单，不含 `oryd/hydra`，换了另一个），拉完标回原名，compose 里
  `名字:tag@sha256:…` 能在本地认出来，安装时 `WONTON_PULL=never`。基础套件约 6.5 分钟。国内首装要带离线镜像
  或者能配镜像站，见“代价与风险”。
- 核心宣告的地址：配对码里是虚拟机自己的局域网地址（`addr=192.0.2.16:8482`），桥接时不用设宣告地址。
- 局域网里另一台设备经隧道（`tunnel-client`）：握手 1–2 ms，路径是直连 `192.0.2.16:8482`（RTT 1.1 ms），
  50 次 HTTP 全部 200，中位 0.9 ms。

### Windows：WSL

- `wsl --install --no-distribution` 从 GitHub 下 367 MB 的安装包，这台机器上 20 分钟没下完；改用微软商店的包
  （`winget install --id 9P9TQF7MRM4R --source msstore`）31 分钟装好 WSL 3.0.1。系统功能事先已开，不用重启。
- 发行版：Debian 13 的根文件系统导入成 `wonton-wsl`（不动全局 `.wslconfig`，发行版自己的 `/etc/wsl.conf` 开 systemd），
  内核串 `6.18.40.1-microsoft-standard-WSL2`；Docker 在 systemd 下正常起。镜像已在本地时装云吞 10 秒，核心宣告的是
  WSL 的 NAT 地址（`addr=172.31.242.243:8482`），局域网里连不到。
- 转发器：宿主上一段 PowerShell 现编的 C#（TCP 8480 和 UDP 8482，UDP 每个来源一个上游套接字，回包从同一个监听端口发回去），
  `netsh portproxy` 只转 TCP，用不上。配对码改成宿主的局域网地址（`192.0.2.17:8482`，即宣告地址）后，同一交换机上的
  另一台设备握手 1 ms，路径直连宿主地址（RTT 1.4 ms），50 次 HTTP 全部 200，中位 1.3 ms。
- WSL 的 NAT 地址每次启动都可能变，转发器每次启动时现查发行版的地址。最后一个 `wsl.exe` 会话退出后 35 秒内发行版就被停掉，
  开着 systemd、容器在跑也一样，启动器要一直挂一个会话（实测用 `sleep infinity`）；再拉起来后容器按重启策略自己起来。
- 宿主内存 16 GB 时，Hyper-V 来宾 2 GiB（云吞基础套件加 magpie 用 545 MB）、WSL 约 1.3 GB，宿主剩 4.7 GB。
  WSL 的内存上限只能在全局 `.wslconfig` 里改，默认是宿主的一半，云吞改不了单个发行版的上限。
- 开机自启（2026-11-08，重启一次）：
  - 两台 Hyper-V 虚拟机按 `AutomaticStartAction Start` 自己起来。
  - WSL 那边靠一个计划任务：触发器是“系统启动时”，以 S4U 方式按用户身份运行、不存密码。它挂起保活会话、现查 WSL 地址、起转发器。
  - 开机后 12 秒任务开始，26 秒拿到 WSL 地址，容器按重启策略全部恢复。
  - 局域网里另一台设备连两台服务器，各 20 次请求全部 200，握手 1–3 ms。
  - 没人登录：主人重启后没碰键盘。
  - 但开机 11 秒后 Windows 自己用这个用户做了一次交互式登录（安全日志 4624，类型 2）；接着 explorer 起来了，看起来是 Windows 在重启后自动登录再锁屏。
  - 开机任务比这次自动登录早 3 秒，以批处理登录（类型 4）启动。所以任务不靠桌面登录，但“开机后连会话都没有”这种状态，这台机器上没法单独测。
  - 系统更新后的重启没有测。
