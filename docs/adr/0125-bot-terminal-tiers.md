# ADR-0125 Bot 终端三档：没终端、旁路容器、危险模式

- 状态：提议（2026-11-26 主人早会定 N9=B 做试点，先在本篇定设计，试点结论回填“验证结果”）
- 取代：[ADR-0036](0036-plugin-privileges.md)“两条不变”第 1 条里“Agent 和 Bot 插件自己拿不到 Docker socket”这句禁令，只对声明了终端服务的插件放开；不动它的分级和“声明了就给”
- 补充：[ADR-0091](0091-danger-mode.md)——危险模式从“唯一的宿主开关”变成第 3 档的必要条件；[ADR-0119](0119-pending-inbox.md)——ask 走它的 `confirm` 条目，不新造通道；[ADR-0110](0110-one-hermes-container-per-person.md)——旁路容器是每 Bot 一组容器里的一个服务；[ADR-0045](0045-family-members.md)——ask 发给这个 Bot 的归属人
- 不动：[ADR-0108](0108-hermes-default-kernel.md)——跟随上游、不改 Hermes 源码；第 2 档是 Hermes 的终端后端插件，不是内核补丁
- 关联：[ADR-0024](0024-confirmation-scope.md)（判定顺序）、[ADR-0016](0016-mcp-bus-and-confirmations.md)（确认卡片和审计）、[ADR-0011](0011-security-model.md)（永远不给宿主 shell 的边界）

## 背景

今天一个 Bot 的终端就一种样子：命令跑在它自己的 Hermes 容器里（`terminal.backend` 默认值）。它和 Bot 的密钥在同一个文件系统里，`cat /opt/data/.env` 就拿到模型 key，`cat /opt/data/config.yaml` 拿到全部配置，`platforms/` 里是 Matrix 令牌。没有任何路径隐藏，模型写的一行命令想读什么读什么。审批只有 Hermes 自己的危险命令启发式（`approvals.mode: smart`），它按命令名猜，不按文件和目录判，`python3 -c "open('/opt/data/.env')"` 不在它的危险名单里。

主人要的是一条能选的阶梯，不是两个极端：

- 有的 Bot（家人的、只查资料的）不需要终端，只走 MCP 工具最省心；
- 有的 Bot 要真 shell，但希望先在这一层把密钥挡住、把删改和装包拦住问一句，而不是直接碰宿主；
- 自己的 Bot 在自己的私聊里要全自动、要能在服务器上动手，就是[危险模式](0091-danger-mode.md)。

第 2 档不能在 Hermes 容器里“就地加隐藏”实现：实测（试点结果见任务目录的 `tasks/mirage-pilot.result.md`）宿主模式（`provider: host`）下只要 profile 里有一条 `paths.hide`，所有外部程序整体被拒（`runtime cannot enforce this session's file restrictions`，exit 126），Bot 连 `python3` 都没有；没有隐藏时程序又能读到整个宿主文件系统。真中间档必须让程序跑在另一个容器里，用 FUSE 把过滤后的视图挂给它。

本篇只定设计、权限和验证方法；试点用 [strukto-ai/mirage](https://github.com/strukto-ai/mirage)（Apache-2.0，Hermes 官方终端后端插件，pin 0.1.0）在开发栈上跑通三档里的第 2 档。

## 决策

### 1. 三档，一个 Bot 一次只能选一个

| 档 | 名字 | 命令在哪跑 | 看得到什么 | 要什么 |
| --- | --- | --- | --- | --- |
| 1 | 没终端 | 不跑程序 | 只有 MCP 工具 | 无 |
| 2 | 旁路容器 | 每个 Bot 一个专用容器 | 过滤后的 Bot 数据视图，密钥默认隐藏 | 插件声明终端服务；容器拿 `/dev/fuse` 和 `SYS_ADMIN` |
| 3 | 宿主终端 | 宿主，以普通用户 | 宿主文件系统 | 已装宿主终端插件；**必须开危险模式** |

- 后端是单值设置 `terminal`（`none`/`sandbox`/`host`），存在 Bot 记录里，改了就重启这个 Bot 的 Hermes，几秒，不动容器和数据。
- 新 Bot 默认第 1 档。存量 Bot 升级后先保持第 2 档上线之前的旧行为（容器内 shell），资料页标“旧式终端”，给一个“换成旁路容器”的按钮；换过去不自动回退（要不要默认迁移见“要主人拍板”）。
- 第 3 档沿用 ADR-0091 的危险模式开关：`terminal: host` 而危险模式没开时，宿主终端上照常每类危险操作发卡片；开着才是私聊里全自动。档 2 与危险模式正交，开着危险模式但同时选档 2，命令仍然在容器里跑、照样问。
- 三档按能力递增，不叠加：不存在“容器里跑不了自动改跑宿主”。

### 2. 谁能开、开在哪

- 开关在 Bot 的资料页“终端”，只有这个 Bot 的归属人能改；家人自己的 Bot 由家人改，主人可以替全家人定“家人默认允许哪一档”（见拍板）。
- 档 3 只给归属人是主人的 Bot（ADR-0091 不变）；家人的 Bot 只能档 1 或档 2。
- 开了档 2，Bot 的资料页和聊天页标题各显示一个“容器终端”徽标；档 3 显示“危险模式”。徽标由核心给 ViewModel，壳只渲染。
- 插件没装、没声明终端服务、宿主缺 `/dev/fuse`、内存不够时，档 2 在设置页置灰并写明原因，不静默落回容器内 shell。

### 3. 档 2 怎么起：一个每 Bot 的终端服务，核心起，不是网关自己起

**谁起。** 核心起，和 `init`/`gateway`/`web` 一样从插件声明的 `bots.services` 里取：插件清单新增 `terminal: { service: terminal }`，核心对每个 Bot 实例化这个服务、随 Bot 建删、随插件卸载清掉。网关不负责起容器，只负责往里 `exec`。服务名固定叫 `terminal`，不允许由用户或插件改名（设置页要按它找）。

**镜像。** 和网关同一个镜像，不另建一套：官方 `nousresearch/hermes-agent` 固定摘要 + 包管理器装的 `mirage-ai[...]`（fork 的 tag）和 `@struktoai/mirage-fuse`（npm，固定版本）+ `fuse3`。网关镜像里已经有 Hermes 和这个后端插件；终端镜像只需要 `sh`、`mirage-fuse`、`fuse3`，同一份镜像意味着两边不会漂版本。镜像摘要写进清单，升级跟插件发版走。

**权限，只在这一档的终端服务上：**

| 项 | 给谁 | 为什么 |
| --- | --- | --- |
| `devices: ["/dev/fuse:/dev/fuse"]` | 终端服务 | `mirage-fuse` 在容器里挂 FUSE 视图 |
| `cap_add: ["SYS_ADMIN"]` | 终端服务 | 容器里 `mount()` 只认这个能力；能挂 FUSE 就是这个能力给的全部 |
| `/var/run/docker.sock`（可选权限，默认随档 2 打开） | 网关服务 | 网关里的 Mirage 后端要 `docker exec` 进终端容器；Mirage 的 docker 机器只认容器名，不认网络别名 |
| 不挂宿主目录 | — | 视图是 FUSE 挂上来的，不是 bind mount |
| 不给 `network_mode: host`、不给 `privileged`、不给设备、不给 GPU | — | 需要时由插件按既有权限另申请 |

**这一档的诚实结论：不是沙箱。** 它有 `/dev/fuse` + `SYS_ADMIN` + docker socket：有 socket 就能 `docker run -v /:/host`，所以“看不见宿主”是默认视野，不是强制。挡的是事故和顺手读到密钥，不是蓄意越狱；要真隔离走档 3 之外的专项（gVisor 之类）再单独立案。下一步可以把 socket 换成只放行 `exec`/`inspect` 的回环代理，把 `docker run` 关掉（见“没选的做法”）。

**视图和默认隐藏。** 视图用 Mirage 的 `mount_type: mirage`：网关是“机器宿主”，程序跑在终端容器里，文件读写全部经过网关的 FUSE 服务，隐藏规则在网关侧判定，容器里的程序一样被挡（试点实测 `python3` 读隐藏路径是 `Permission denied`）。默认：

- 可写挂载只有 `<Bot 数据目录>/work`（同路径挂进视图），它是唯一的工作区，随 Bot 数据保留；
- 默认隐藏：`/opt/data/.env`、`/opt/data/config.yaml`、`/opt/data/platforms/**`（Matrix 令牌和加密存储）、`/opt/data/plugins/**`（插件的密钥）、`/opt/data/cache/**`、`/opt/data/logs/**`；工作区里按名字隐藏 `*.env`、`*.key`、`*.pem`、`id_*`、`.netrc`、`.npmrc`、`.pypirc`、`.git/config`、`credentials.json`；
- `paths.show` 只放回 `work` 本身；工作区外的读取不隐藏时仍可读（`/usr`、`/opt/hermes` 等程序目录），但不给写；
- 模板由插件随 Bot 初始化写入数据目录，主人可以在 Bot 数据目录里改（YAML 原样生效），设置页不暴露编辑器。

**资源上限。** 终端服务固定 `mem_limit: 256m`、`pids_limit: 256`、`restart: unless-stopped`、无 GPU；超了就是 OOM 杀进程 / 杀容器，命令如实失败并在 Bot 状态里标“终端容器被杀”，不悄悄重试。工作区在宿主机上不设配额，按 Bot 数据目录已有的规则管。

**网络。** 终端服务只进插件网络、出网经插件声明过的出口；不开宿主网络；不进核心网络以外的东西。`pip install` 装到容器层，重建即丢，工作区要留就把包装进项目目录。

**失败表现。** 终端容器没起好：终端工具返回“终端容器不可用（原因）”，不发卡片、不落到容器内 shell；FUSE 没挂上：第一次命令就报 `fusermount3` 的原因；docker socket 没给：第一次命令报 `docker` 不可达。都写进 Bot 状态和日志。

### 4. 命令规则和 ask：默认问哪几类，问到哪

- 模板默认：`"*": allow`；`rm`、`mv`、`pip`、`npm install`、`apt`、`curl`、`wget`、`git push`、`ssh` 是 `ask`；`sudo`、`systemctl`、`mount` 是 `deny`（容器里没有它们，问了也白问）。
- 已知缺口照实写：规则按命令名精确匹配，`python3 -m pip` 绕过 `pip`；模板给 `python3` 的 `-m pip` 子命令也写一条。后续要不要按 argv 前缀匹配另议。
- **一个命令最多一张卡。** 配了 ask 端点时，这个后端的 `skip_container_guards = True`，Hermes 自己的危险命令守卫让位（实测两套一起开会对同一条命令连发两张卡）；非终端工具（网页、发信）仍归 Hermes 的审批。
- ask 走核心已有的 `POST /agent/permissions`（ADR-0035 留下的 Agent 权限入口，长请求、600 秒 TTL、审计）：wonton 插件在网关里开一个只监听回环的桥，把 Mirage 的回调翻成一次 Agent 权限请求——`tool_call_id` 用 ask id，`title` 是“运行 `<命令行>`”，`arguments` 带 argv、cwd、reason，选项是“这次允许”“本会话允许”“拒绝”“总是拒绝”。核心把它变成收件箱里的一条 `confirm` 条目（ADR-0119），发给这个 Bot 的归属人，App 在发起的会话里就地作答。
- 映射：这次允许 → `allow`/`once`；本会话允许 → `allow`/`session`（本会话内同一条规则不再问）；拒绝和总是拒绝 → `deny`；“取消”和核心侧过期 → `deny`。
- 端点不可达、返回非 2xx、回包读不懂、超时，一律 `deny` 并把原因写进记录，绝不默认放行。Mirage 侧超时默认 120 秒（设置项，30–600），必须短于核心 600 秒 TTL，让“没人答”在卡片过期之前就落成 deny。
- 危险模式开着、命令又落在档 2 时，ask 照样发卡片：危险模式管的是“不问”，档 2 的卡片是“换了个地方问”。

### 5. 清单和规范的改动

- 清单新增顶层 `terminal: { service: terminal }`：`service` 必须是 `bots.services` 里的一项，写别的服务、写不存在的服务都不合法。核心靠它找终端服务，设置页和状态也靠它。
- 修订 ADR-0036 的“两条不变”第 1 条：声明了 `terminal` 的插件，**只在这一个服务上**可以申请 `devices./dev/fuse`（高，本来就可以）、`cap.SYS_ADMIN`（最高）和 `docker.socket`（最高，可选、默认随档 2 打开）；别的插件仍然不能申请很高和最高级。风险级别不变，安装确认照旧把两项最高风险单独醒目列出，并加一句“给这个 Bot 的终端容器”。
- ADR-0036 的“两条不变”改成三条：宿主 shell 仍然没有；容器内 shell 不等于宿主，终端容器的 `SYS_ADMIN` 只用来挂 FUSE；`SYS_ADMIN` 和 socket 只加在终端服务、网关只拿 socket。
- 校验沿用现有成对规则：清单声明权限、compose 写字段、核心校验两边对得上；`cap.SYS_ADMIN`/`docker.socket` 出现在别的服务上、或 `terminal.service` 不在 `bots.services` 里，清单不合法。
- 契约（新能力检查单）：ask 的线上格式是 HTTP JSON（见第 4 节），核心侧是 `AgentPermissionRequest`；提供方是 Mirage 的 fork（`ask_url` 回调）；消费方是 Hermes 的终端后端。换提供方时消费方不用改。

### 6. 审计和可见性

- 每次 ask 写核心的 Agent 权限审计（谁、哪条命令、什么参数、答了什么、谁答的）和 Bot 自己 `decisions` 的磁盘日志；Mirage 的决策存储从默认的内存改为磁盘，重启不丢。
- Bot 的终端档、终端容器状态（运行/停止/被杀/不可用）进 Bot 状态 ViewModel，资料页和“等你确认”列表可见。
- 终端容器不写宿主日志，stdout/stderr 由 Mirage 收回；容器自己的日志只给运维看。

### 7. 为什么是 Mirage，不是自己写或别的沙箱

| 方案 | 有的 | 缺的 |
| --- | --- | --- |
| Hermes 容器内 shell（今天） | 零成本 | 密钥同 FS、无隐藏、无命令规则、无 ask |
| 自写 `docker exec` 后端 | 能隔离 FS | 自己写视图、隐藏、规则、ask、机器抽象；四件事都要维护 |
| bubblewrap / firejail | 本机隔离 | 不跨容器；没有查看器和 ask 协议；要自己包一层 shell |
| gVisor / Firecracker | 真沙箱 | 新运行时、镜像兼容性、内存底价；仍然没有文件视图和 ask |
| Hermes 自带 `approvals` | 有危险命令启发式 | 只看命令名不看文件；没有视图；问只问在本进程里 |
| **Mirage** | 视图、隐藏、命令规则、ask 回调、docker/ssh 机器，Apache-2.0，Hermes 官方后端插件 | alpha、FUSE 依赖、`SYS_ADMIN`；ask 没有外部通道，要我们补一个可上游的补丁 |

选它的理由是它把这一档需要的四件事都做成了现成件，Wonton 只补 ask 的回调通道这一个缺口，补丁按上游风格写、不 fork 行为。不选自己写，是因为这四件事没有一件小到可以在插件里顺手写完还不用长期维护。

## 理由

- 三档按“能碰什么”分，不按“问不问”分；问不问留在确认和危险模式里，两个维度不搅在一起。
- 中间档必须真隔离文件视野，否则和今天的容器内 shell 没区别；真隔离就得让程序跑在另一个容器里。
- 容器由核心按 Bot 起，和现有 Bot 生命周期一致，插件不发明第二种容器管理。
- 权限只加在终端服务上，网关拿到的只有 socket；`SYS_ADMIN` 不摊给整个插件。
- ask 走已有的 Agent 权限入口和收件箱，不新造第五种确认通道。

## 代价与风险

- **不是沙箱**：同内核、`SYS_ADMIN`、docker socket。恶意模型能逃；挡的是事故和顺手读密钥。
- **docker socket 等于宿主 root**：这是这一档最大的让步，安装确认必须写明。缓解见“没选的做法”。
- **alpha**：Mirage 0.1.0、内部 API 动得快；钉 tag 和摘要，升级前跑整套冒烟。
- **FUSE 依赖**：宿主内核要有 fuse；`/dev/fuse` 在 LXC、部分 NAS、群晖上默认不可用，档 2 在这些机器上不可选。
- **性能**：每次外部程序调用多 40–50 ms（docker exec + FUSE，试点实测中位 47 ms），交互式的 `ls`/`cat` 由内置命令走，不受影响；重脚本会慢。
- **每个开档的 Bot 多一个容器**：内存底价 + 上限 256 MiB；家人 Bot 全开时主机内存要按 ADR-0110 的口径核对。
- **Hermes 守卫让位**：配了 ask 端点时 Hermes 自己的危险命令守卫不再对终端命令生效，全靠模板规则；模板漏的类目会直接放行。缺省模板要保守，改动要过审。
- **路径隐藏是网关侧判的**：端点或网关进程挂了以后，已经起来的终端会话行为要测（待验证 5）。

## 没选的做法

- **在 Hermes 容器里就地隐藏**：任何一条隐藏都让外部程序 126，实测，不可用。
- **`provider: host` 加 qualify**：还是同 FS，隐藏挡不住 `python3`。
- **靠 ssh 传输替代 docker socket**：sidecar 要跑 sshd 和 sshfs，网关也要 sshd，两个方向都要钥匙；能去掉 socket，但多两个守护进程和一份未验证的隐藏路径（待验证 4）。
- **把 `SYS_ADMIN` 摊给整个插件**：只在终端服务上给，网关保持干净。
- **特权容器换 `SYS_ADMIN`**：特权还带别的能力和设备，超过需要。
- **让危险模式顺手等于档 2**：危险模式是“不问”，档 2 是“换地方”，混成一个开关会让家人 Bot 也跟着全自动。

## 要主人拍板

1. **存量 Bot 要不要在一次升级里默认迁到档 2？** 推荐：不自动迁，资料页给按钮、点一次迁一个；迁完回不去旧式是干净，但要多点一次。
2. **家人默认档**：推荐家人 Bot 默认档 1，主人可为全家人打开档 2；档 2 的卡片发给 Bot 归属人（家人自己），不发给主人，主人只在“等你确认”里看到管理员视图。
3. **ask 默认超时**：推荐 120 秒；等不到就按拒绝。要不要给“主人自己的 Bot 永不过期”的选项？
4. **`docker.socket` 的可选默认**：推荐随档 2 默认开；要不要先做只放行 exec 的回环代理再默认开？（多一个容器，工作量约两天）
5. **命令规则模板的默认值**：`curl`/`wget` 是 ask 还是 allow？ask 能防下载执行，代价是每次取资料都点一下。

## 待验证

1. 隐藏对容器里的 `python3` 真生效（试点已测单条路径；工作区通配和多条隐藏要补测）。
2. 同一命令只出一张卡：配了 ask 时 Hermes 守卫真让位。
3. ask 的四种回答映射、超时、不可达、回读不懂都落到 deny，且审计有记录。
4. ssh 传输替代 docker socket 时，隐藏和 ask 行为不变。
5. 网关进程重启 / ask 桥挂掉后，已在跑的会话里的 ask 和文件读写行为。
6. 终端容器 OOM、被外部删掉、`/dev/fuse` 不存在的三种失败在 UI 上的文案和恢复路径。
7. 家人 Bot 的 ask 只发给归属人；管理员视图能看到。
8. 变更档位时的重启时延和对正在跑的会话的影响。

## 验证结果

（试点完成后回填：命令、输出和 `tasks/mirage-pilot.result.md` 的路径。）
