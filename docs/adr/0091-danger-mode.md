# ADR-0091 危险模式：主人私聊里全自动，宿主终端以普通用户执行，sudo 密码由核心保管

- 状态：提议（2026-11-12）。取舍由主人定（见“已定”）
- 取代：[ADR-0011](0011-security-model.md) “已定”里的“永远不给 Bot 宿主的 shell 和 Docker”；改写 [ADR-0036](0036-plugin-privileges.md) 的“两条不变”和“最高级插件的工具不能放宽”；
  补充 [ADR-0024](0024-confirmation-scope.md) 的判定顺序和 [ADR-0035](0035-acp-bridge.md) 的权限请求

## 背景

ADR-0011 定了永远不给 Bot 宿主 shell；ADR-0024 的放宽最多到“一律允许”，核心自己的敏感工具、最高级插件的工具，以及删东西、装应用、
开放到外面这三类（ADR-0024“每次都问的三类”）每次都发卡片。主人要的是另一种用法：在自己的私聊里把活交给 Bot，它能直接在服务器上动手，
中途不再弹卡片。

主人 2026-11-12 定了要做，并定了身份、范围和谁能开（原话见“已定”）。别人的做法（Claude Code 的 bypassPermissions、Codex 的
`danger-full-access`、OpenClaw 的 elevated、Hermes 的 `/yolo`、Home Assistant 的保护模式）都把“问不问”和“碰得到什么”分成两件事。
这里也分：问不问由核心的一个开关管，碰得到什么由一个独立的宿主终端插件管。

## 决策

### 1. 开关

- **一个 Bot 一个开关，默认关，存盘。** 和“一律允许”存在同一个 `core/plugins/tool-policy/policy.json`（ADR-0024），新键 `danger`，
  值是开着的调用方。核心重启后保持；没有时长分档。Bot 被删时一起删。
- **只有主人能开。** 打开只接受经隧道的已配对设备，而且不是网页发起的请求（ADR-0017 的那道门），不接受主人令牌、Cookie 会话：
  主人原话是开关要“在 App 里点”。关掉接受同一道门，也接受主人令牌，主人在服务器上也能紧急关掉。Agent 和插件没有任何接口能开。
- **只给归属人是主人的 Bot**（ADR-0045）。家人的私人 Bot、共享 Bot、电脑上的 Agent（[ADR-0081](0081-local-agent-host.md)）不显示开关，
  打开请求回 400 `invalid_input`。共享 Bot 的同一个 Agent 进程也接家人的消息，见“没选的做法”。
- **打开要两步。** 第一步是开关，第二步弹出和 ADR-0036 最高级同一档的提示，要点“我知道这个 Bot 能以下面的身份控制这台主机”。提示由核心给，
  写明：开着时 Bot 在你私聊里做的事都不再问，包括删东西、装应用、开放到外面；能碰宿主时以哪个账号、有没有 sudo；网页、文件、邮件、
  别的 Bot 里夹带的指令也能让它这样做，危险模式挡不住提示注入。
- **开着时有醒目标识。** Bot 资料页和聊天页标题处显示“危险模式”，由核心在 Bot 的 ViewModel 里给。
- **没有能碰宿主的插件时如实写。** 这个 Bot 看得到的插件里没有最高级权限且有 `mcp` 的，开关旁写“没装能碰宿主的插件，只在容器和 Wonton 里全自动”。

### 2. 判定只在核心一处

所有卡片都从核心确认表的同一个入口生出来（`mcp/confirm.rs` 的 `Confirmations::open`：MCP 插件工具、核心自己的敏感工具、
Bot 装插件的两步卡片、ACP 权限请求都经过它）。危险模式的判定放在这里，发卡之前问一次，三个条件都满足才不发卡片：

1. 发起的调用方是 Bot，它的危险模式开着；
2. 这次请求的来源会话是主人和这个 Bot 的私聊（第 3 节）；
3. 卡片是“同意或拒绝”一类：
   - 带输入的卡片照常发：要主人填密钥的安装（ADR-0064）、多步问题卡片（ADR-0038）、登录。核心替主人填不了，也不该填。
   - ACP 权限请求只在选项里有 `allow_once` 时代答，选它；只有 `allow_always` 时照常发卡片。永远不选 `allow_always`：那会写进 Agent 自己的配置，
     关掉危险模式后还生效。

满足时核心不建卡片，直接当作主人同意：MCP 调用照常转给插件，ACP 请求回 `selected` 和 `allow_once` 那项的 `choice_id`。审计照常写（第 7 节）。

**危险模式压过所有“每次都问”。** ADR-0024 判定的第 1 步（核心自己的敏感工具）、最高级插件的工具（ADR-0036）、每次都问的三类，
在上面三个条件都满足时都不再发卡片。主人原话：“删、装、开外网”三类也不弹。危险模式是它们唯一的例外，写成测试；同时测危险模式关着、
来源不是私聊、来源落到主房间时，它们照旧每次发卡片。

不变的：

- “我来操作”（ADR-0024）照常先停住 Bot 的调用，它不是确认，是免得 Bot 和主人抢同一个页面。
- 工具看不看得到、调不调得了（插件的 `mcp.tools`、按 Bot 的范围和单独关掉，ADR-0066）照旧，危险模式只管问不问。
- 不切 Agent 的 bypass、yolo、full-access，不改 Agent 镜像的设置（ADR-0035 的“最严格模式”不变）。Agent 照常发权限请求，由核心代答。
- 桥不感知危险模式。它照常把权限请求交给核心，收到的回答和主人点了 `allow_once` 一样。

### 3. 怎么认出“主人私聊里的消息”

用核心已有的来源会话（ADR-0016 第 2 步），只加一条更严的要求：

- **来源从哪来。** MCP 调用是 `params._meta["app.wonton/origin"]`，或固定请求头 `wonton-origin-room`、`wonton-origin-thread`，两样都有时 `_meta` 为准；
  ACP 权限请求是请求里的 `origin` 字段（`wonton-protocol` 的 `confirmation_origin`、`agent_permission`）。ACP 桥在 `session/new` 时按会话放请求头，
  一个房间一个 ACP 会话，所以头和房间一一对应。
- **算私聊的条件。** 来源是 Agent 一侧报的、核心按 ADR-0016 核对过的；房间里已加入的成员恰好是这个 Bot 和主人两个（以 Bot 的身份查 `joined_members`，
  每次判定时查，不用缓存）。私聊里的话题也算。
- **不算的。** 没报来源、核对不上、查不了，按 ADR-0016 落到 Bot 主房间的，**不算**：落到主房间只决定卡片显示在哪，不能证明消息来自私聊。
  群、联系人房间（ADR-0045 的 Bot 互发）、有好友或第三个成员的房间都不算。不报来源的 Agent（如 Hermes 用一份固定配置）因此在危险模式下照常发卡片，
  这是如实的结果，不为它特殊处理。
- **它挡什么、挡不住什么。** 请求头是桥按会话写的，`_meta` 由 Agent 程序填，模型改不了；群里的提示注入要伪造来源，得让 Agent 自己发一个带假头的
  HTTP 请求，这需要容器里的 shell 或能自定请求头的工具，而群里会话的这类操作走 ACP 权限请求、来源是群，照常发卡片。所以这是便宜的一道门，
  挡住经群、好友、别的 Bot 来的指令，不是边界：Agent 不问就能发 HTTP 请求的话（某些 Agent 的抓网页工具），来源可以伪造（见“代价与风险”）。

### 4. 宿主终端插件

能碰宿主是一个独立插件（`wonton-plugin-host-terminal`，公开仓库），核心不认识它：

- **权限。** 最高级：`privileged`、`host.pid`，加新权限 `host.sudo`（第 5 节）。它不是 Agent 插件，ADR-0036 的清单校验不用改。
  危险模式关着时，它的工具和别的最高级插件一样每次发卡片。
- **怎么进宿主、以谁的身份。** 插件的执行助手以容器里的 root 起步，`setns` 进 PID 1 的 mount、UTS、IPC、net、pid 命名空间（同
  `nsenter -t 1 -m -u -i -n -p`），再 `initgroups`、`setgid`、`setuid` 切到选定的宿主用户，清空环境后按该用户的 passwd 设 `HOME`、`USER`、`SHELL`、
  `PATH`，在家目录里 `exec` 他的 shell。等于 `nsenter … -- setpriv --reuid --regid --init-groups --reset-env`。不开 sshd，不改宿主任何配置。
  - 只用 nsenter 自带的 `--setuid`、`--setgid` 不够：它会清掉附加组，`%sudo`、`%wheel` 这类 sudoers 规则按进程的组匹配（sudo.conf 的
    `group_source` 默认 `static`），用户就用不了 sudo，也丢了 `docker` 这类组。所以要 `initgroups`。（由 sudo 文档推断，实现时实测。）
  - 宿主用户是插件设置（[ADR-0077](0077-plugin-settings.md)）里的一项，必填，没有默认值；助手按宿主的 `/etc/passwd` 核对，不存在或 UID 是 0 时插件不健康，
    写明原因。主人要的是普通用户，不给选 root。
- **MCP 服务优先包上游。** 首选 [DesktopCommanderMCP](https://github.com/wonderwhy-er/DesktopCommanderMCP)：它的 `start_process` 收 `shell` 参数，
  配置里有 `defaultShell`，不认识的 shell 交给 Node 的 `spawn(…, {shell})`，即 `<shell> -c <命令>`。镜像把执行助手配成默认 shell，
  清单用 `mcp.tools` 只放出进程类工具（`start_process`、`read_process_output`、`interact_with_process`、`force_terminate`、`list_sessions`），
  不放它的文件工具：那些在容器里用 Node 的 `fs` 直接读写，碰到的是容器，不是宿主。读写宿主文件走命令。`telemetryEnabled` 关掉。
  它只说 stdio，镜像里用 supergateway（MIT，ADR-0016 已在用）转成 Streamable HTTP。
  - 备选 [tumf/mcp-shell-server](https://github.com/tumf/mcp-shell-server)：只有一个执行工具，按 `ALLOW_COMMANDS` 白名单 `exec` 程序，
    要把助手做成白名单里唯一的程序，Agent 写起来别扭。首选在实测里走不通时再换。
  - 许可证（2026-11-12 经 GitHub API 读 `LICENSE` 原文）：DesktopCommanderMCP 是 MIT（“Copyright (c) 2024-2025 Eduard Ruzga and Desktop Commander
    Contributors”，`package.json` 的 `license` 也是 MIT，最新 v0.3.2）；tumf/mcp-shell-server 是 MIT（最新 v1.1.14）。插件按 AGENTS.md 固定一个
    发布满两周的版本，入库前在那个版本的标签上重读 `LICENSE` 和 `package.json`（或 `pyproject.toml`），依赖的许可证用
    `npx license-checker --production`（或 `pip-licenses`）列出，没有 GPL/AGPL 才用；结果写进插件的 NOTICE。
- **助手是唯一跨进宿主的部分。** 上游 MCP 服务在容器里以无特权的用户运行，没有能力（capabilities）；只有执行助手能进宿主、能读 sudo 文件。
  Agent 给 `shell` 参数指定别的程序时，那个程序以这个无特权用户跑在容器里，碰不到宿主。助手做成 setuid root 的程序还是 root 的旁路服务（经 unix
  套接字），实现时实测后选，两种都要满足这一条。
- **在 Mac、Windows 上**，宿主是云吞自带的那台 Linux 虚拟机（[ADR-0062](0062-desktop-vm-host.md)），不是用户的电脑。

### 5. sudo 密码

- **在哪填。** 危险模式设置里有一项“宿主终端账号的 sudo 密码”，整台服务器一份（宿主终端只有一个账号），所有开了危险模式的 Bot 共用。
  用系统密码输入框，和 ADR-0064 的输入密钥同一套：只有主人能填（打开开关的那道门），任何接口都不回显，只给“已填写”和修改时间，审计只记改了。
  旁边如实写：“只有这个账号本来就在 sudoers 里才有用，Wonton 不改宿主的 sudoers。不填就没有 sudo。”
- **存在哪。** 核心密钥库（核心数据目录，0600，插件容器访问不到），不进 Matrix、聊天记录、日志、审计。
- **怎么交给插件。** 声明了 `host.sudo` 的服务，核心把密码作为只读文件 `/run/wonton/host-sudo`（属主 root，0400）挂进容器。不用环境变量：
  环境变量会被子进程继承，一路带到 Agent 跑的命令里。文件只在两件事同时成立时存在：密码已填，且至少一个 Bot 开着危险模式。任一条变了，核心重建这些服务。
- **怎么用。** 宿主终端多一个 shell：`host-sudo-sh`。Agent 要 root 时在 `start_process` 的 `shell` 里选它。助手以 root 读出文件，建一个管道写进
  `密码\n` 后关掉写端，把读端接成标准输入，再进宿主、切到用户，`exec sudo -S -k -p '' -- <用户的 shell> -c <命令>`。
  - `-k` 不用也不留 sudo 的凭据缓存，后面不带密码的 `sudo` 照样要密码。
  - 密码只经过助手和 sudo 的标准输入，不在命令行里（`ps` 看不到），不在环境里，不回到 MCP 结果。这个 shell 里命令的标准输入在密码之后就结束，不能交互。
  - 没有文件时，`host-sudo-sh` 不起进程，直接报“没有 sudo 密码，或者危险模式没开”；sudo 拒绝时（密码错、不在 sudoers）照 sudo 的输出报错，不当成功。
- **Agent 和聊天看不到它**：在 Agent 拿到 root 之前。拿到 root 后它能读这个文件和核心的密钥库，见“代价与风险”。

### 6. 收回

主人关掉一个 Bot 的危险模式（或清空 sudo 密码）时：

1. 核心立刻按关着判定：这个 Bot 下一次调用、下一个 ACP 权限请求照常发卡片。
2. 这个 Bot 正在执行、因危险模式放行的 MCP 调用，核心发 MCP 取消（`notifications/cancelled`）。插件规范要求 `host.sudo` 的插件收到取消时结束这次命令的整个进程组。
3. 没有 Bot 开着危险模式了，或者密码清空了，核心按第 5 节重建声明了 `host.sudo` 的服务：sudo 文件消失，容器里的进程随容器结束。
   经 `nsenter -p` 进了宿主 pid 命名空间的进程是否随容器的 cgroup 一起结束，待实测；用 `systemd-run`、`nohup`、`setsid`、cron 交给宿主的，肯定留下。
4. ACP Agent 已经以 `allow_once` 开始的操作归 Agent 自己，核心停不了；下一次权限请求开始照常发卡片。

关掉不收回已经做完的事。Agent 用 sudo 拿到过 root 的话，关开关只在它没改过宿主和 Wonton 时有效。

### 7. 审计

`core/audit.jsonl` 照常每次一行，`basis` 多一个 `danger`，带核对过的来源房间。ACP 权限请求的代答也写一行：调用方、`tool_call_id`、选的选项种类。
开关的开和关、sudo 密码的填写和清空各写一行，带哪台设备或主人令牌。审计写不进去时照 ADR-0024 只记错误日志。

### 8. 接口

- `wonton-protocol`：Bot 的 ViewModel 加危险模式的状态（开没开、能不能开和不能开的类别、有没有能碰宿主的插件）；工具策略加 `danger`；
  sudo 密码的“已填写”状态；接口路径常量：`PUT`、`DELETE /_wonton/api/plugins/{id}/tool-policy/danger/{caller}`（`{id}` 是 Bot 所在的插件，同“一律允许”），
  `PUT`、`DELETE /_wonton/api/danger/sudo-password`，`GET /_wonton/api/danger`。不认识的类别落到 `Other`。**动接口，先合。**
- 插件规范（wonton-docs）：新权限 `host.sudo`，最高级，只能和 `privileged`、`host.pid` 一起声明；文件路径、出现条件、取消时结束进程组。
  规范仓库的 PR 在核心 PR 合并后再合。
- ACP 桥、`/agent/permissions` 的接口不变。

## 理由

- 问不问和碰得到什么分开：开关只改核心要不要问，宿主能力是装不装一个插件。不装宿主终端时，危险模式只是容器里和 Wonton 里的全自动。
- 判定放在卡片出生的地方，MCP、核心工具、ACP 三条路不用各写一份，也不会漏掉以后新加的卡片。桥和 Agent 都不用知道危险模式（ADR-0015）。
- 用普通用户而不是 root：主人原话“普通用户但是如果给密码可以 sudo”。sudo 由主人给密码才有，权力的上限由宿主上这个账号本来的权限决定，主人看得懂。
- nsenter 而不是 SSH：只要 Docker，不用宿主装 sshd、建账号、改 sudoers，安装脚本不碰宿主（ADR-0002）。
- 密码经文件给助手、经管道给 sudo：AI 拿到的是“能以 root 跑命令”，不是密码本身；关掉危险模式时文件消失，sudo 跟着没了。

## 没选的做法

- **直接给 root（`nsenter` 不切用户）。** 最省事，主人选了普通用户加可选的 sudo。
- **SSH 到宿主用户**（Hermes 的 ssh 后端）。要 sshd、建用户、配 sudoers，安装脚本得改宿主。
- **给 Agent 容器 `privileged`**（Home Assistant 的 full_access）。要改 ADR-0036 的清单校验；开关一次就要重建容器、会话断；宿主命令不经核心，没有逐条审计，也关不掉。
- **切 Agent 的 bypass、yolo 模式。** 每家写法不同，核心或桥要认识具体 Agent；Claude 以 root 运行时本来就拒绝 bypass。代答 `allow_once` 对所有 ACP Agent 一样。
- **代答 `allow_always`。** 写进 Agent 自己的规则，关掉危险模式后还生效。
- **按来源落到主房间也算私聊。** 不报来源的 Agent 什么都会落到主房间，等于不看来源。
- **每次调用把密码放进 `_meta` 交给插件。** 能按 Bot 精确收回，但 DesktopCommander 不把 `_meta` 交给 shell，要自己写一个 MCP 服务。
- **`SUDO_ASKPASS`。** askpass 程序以这个用户的身份运行，AI 也是这个用户，能自己设一个 askpass 把密码要出来。
- **密码按 Bot 各存一份。** 宿主终端只有一个账号，同一个密码存几份没有意义。
- **共享 Bot 也能开。** 它的 Agent 进程同时接家人的消息，家人那边一次注入就能借来源伪造（第 3 节）碰到主人的宿主。先只给主人自己的 Bot。
- **时长分档、结束时汇总、宿主终端里的底线名单。** 主人定先不做，审计里都查得到。底线名单只挡失手，换个写法就绕过。
- **分类器代审**（Claude Code 的 auto、Cursor 的 Auto-review）。它们的文档自己说不是安全边界，又要一个模型。

## 代价与风险

这些危险模式护不住，开关的提示里写明：

- **AI 能做这个宿主账号能做的一切。** 读写他的全部文件和 `~/.ssh`、用他的 `docker` 组（等于 root）、装 `systemd --user` 服务和 crontab、往外连网。
  读到的内容会发给模型服务商。
- **填了 sudo 密码，就是宿主 root。** 能读核心的密钥库、主人令牌和 sudo 密码文件，能改 `policy.json` 让危险模式一直开着，能改审计、停掉核心。
  关开关只在它没动手脚时有效。
- **挡不住提示注入。** 主人私聊里转来的文字、Bot 读到的网页、邮件、文件和工具结果里夹带的指令，都会被全自动执行。“只认私聊”只挡经群、好友、别的 Bot 来的消息。
- **来源能被伪造。** Agent 能不经权限请求发自定请求头的 HTTP 请求时，群里会话的注入能把来源报成私聊（第 3 节）。
- **收回不彻底。** 交给宿主 systemd、cron 或脱离进程组的进程，关开关后照样跑。
- **sudo 文件在时，任何 Bot 经卡片批准的宿主终端调用都能选 `host-sudo-sh`。** 只开了 A 的危险模式，B 的调用仍要主人点卡片，卡片上写着最高级的“以本机 root 身份执行”。
- **主人不看提示就开。** 只有两步确认和醒目标识。

## 待验证

- 宿主上 `initgroups` 后 `%sudo` 规则生效；只用 `--setuid`/`--setgid` 时 sudo 失败（核对第 4 节的推断）。
- DesktopCommanderMCP 固定版本里，`shell` 参数和 `defaultShell` 指向助手时确实以 `<shell> -c <命令>` 启动；`force_terminate` 和 MCP 取消能结束进程组。
- 关掉后重建容器，`nsenter -p` 进了宿主 pid 命名空间的进程会不会一起结束。
- rootless Docker 下 `privileged` 加 `host.pid` 能不能进到真正的宿主；不能时插件如实报未就绪。
- 各家 ACP 适配器（Claude、Codex、omp、Qwen）收到核心代答的 `allow_once` 的行为，尤其 Codex。
- Mac、Windows 的虚拟机里有没有可选的普通用户。

## 已定

主人 2026-11-12 定：

1. **身份。** 原话：“普通用户但是如果给密码可以 sudo”。宿主终端经 nsenter 进宿主后切到选定的普通用户，不开 SSH、不改宿主配置；sudo 密码在危险模式设置里存一次，
   进核心密钥库，插件用 `sudo -S` 喂密码，AI 和聊天记录都看不到；不填就没有 sudo，清空或关掉危险模式即收回；前提是这个用户本来就在 sudoers 里，界面上如实写。
2. **开着时全自动。** 连“删、装、开外网”三类也不弹卡片，只有开关危险模式本身要主人在 App 里点。
3. **只对主人私聊里的消息生效。** 群里、好友、别的 Bot 的消息照常弹卡片。
4. **只有主人能开**（家人账号那边也是这么定的），一个 Bot 一个开关，默认关。
5. **先不做**：时长分档、结束汇总、底线名单；电脑上的 Agent（ADR-0081）先不给。
6. **判定只在核心一处。** ACP 的权限请求由核心代答“允许一次”，桥不感知危险模式。

这几条推翻了 ADR-0011 “已定”里主人 2026-11-04 定的“永远不给 Bot 宿主的 shell 和 Docker”。宿主 shell 现在经宿主终端插件给，危险模式关着时每次发卡片；
Docker 仍不直接给 Bot，宿主账号在 `docker` 组里时等于给了，提示里写明。

## 关联

[ADR-0011](0011-security-model.md)、[ADR-0015](0015-small-components-standard-interfaces.md)、[ADR-0016](0016-mcp-bus-and-confirmations.md)、
[ADR-0017](0017-plugin-ecosystem-and-store.md)、[ADR-0024](0024-confirmation-scope.md)、[ADR-0035](0035-acp-bridge.md)、[ADR-0036](0036-plugin-privileges.md)、
[ADR-0045](0045-family-members.md)、[ADR-0062](0062-desktop-vm-host.md)、[ADR-0064](0064-input-secrets.md)、[ADR-0066](0066-plugin-forms.md)、
[ADR-0077](0077-plugin-settings.md)、[ADR-0081](0081-local-agent-host.md)
