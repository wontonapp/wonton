# ADR-0036 插件的高权限：声明了就给，按风险分三级提示，由主人决定

- 状态：已接受（2026-11-04）。方向由主人确认（见“已定”）。最高级“多点一步”在确认卡片上连发两张，合成一张两步卡片的提议见 [ADR-0038](0038-multi-step-cards.md)。“两条不变”和“最高级插件的工具不能放宽”由 [ADR-0091](0091-danger-mode.md)（危险模式）改写；“设备”一节 2026-11-12 重写（主人定，见“已定”）：USB 改按类、串口拔插后自动恢复、新增硬盘一类
- 取代：[ADR-0011](0011-security-model.md) “插件权限”里的“host 网络、Docker socket、特权模式一律不开放”和“网络暴露”里“例外有两个”；[ADR-0019](0019-lan-services-for-plugins.md) 的“`network.host` 仍不开放”；[ADR-0006](0006-plugin-system.md) 的“高权限字段在 M0 一律拒绝”；[ADR-0033](0033-plugin-tabs.md) “确认”里“`network.host`、`docker.socket` 规范里对谁都不开放”

## 背景

规范拒绝了宿主网络、`cap_add`、特权模式、Docker socket 和显卡以外的设备。结果是一批主流应用装不上，或者装上了不能用：

| 应用 | 要什么 | 没有它会怎样 |
| --- | --- | --- |
| Home Assistant | 宿主网络（mDNS、SSDP、DHCP 发现），USB 或串口的 Zigbee 棒 | 发现不了局域网设备，这是它的主要价值 |
| Pi-hole、AdGuard Home 的 DHCP | `NET_ADMIN`，宿主网络 | 只能做 DNS |
| Zigbee2MQTT、ESPHome、UPS 监控 | USB、串口 | 完全用不了 |
| DLNA、Syncthing 的本地发现 | 组播、广播 | 只能手填地址，DLNA 完全用不了 |
| Portainer、Dockge | Docker socket | 完全用不了 |
| Netdata、Glances | `pid: host` 等 | 只看得到自己 |

主人 2026-11-04 定：“该放开放开，很多限制会导致完全用不了或者非常受限的。”ADR-0011 本来就写着“确实有风险、拦了又伤体验的，做成开关，写明风险，主人自己打开”，这一篇把插件权限也改成照它来。

别人的做法：Home Assistant OS 的 add-on 清单能声明 `host_network`、`privileged`、`docker_api`、设备，商店按权限显示安全评分；CasaOS 的应用直接写 compose 字段。我们取前者的声明和分级，后者的写法。这段凭对两者的了解写，没有逐条再查。

## 决策

### 原则

- **声明了就给。** 清单声明，安装时按风险提示，主人确认后兑现。核心不替主人判断某个插件配不配。
- **没声明的照旧不给。** ADR-0011 “插件隔离的约束”全部保留：挂载源不能在容器可写的目录上、资源名不重复、浏览器里的隔离。它们管的是“没声明的拿不到”。
- **两条不变：**
  1. Agent 和 Bot 插件自己拿不到 Docker socket、特权模式和宿主 shell：清单校验照旧拒绝它们申请很高和最高级（见“谁拿不到”）。Bot 碰宿主只经
     最高级插件的工具（如宿主终端），平时每次发卡片；主人给这个 Bot 开了危险模式、消息来自主人私聊时不发（[ADR-0091](0091-danger-mode.md)）。
     宿主指服务器；用户自己电脑上的 Agent 见 [ADR-0081](0081-local-agent-host.md)，不给危险模式。
  2. Bot 装任何高权限插件都走确认卡片（ADR-0033），危险模式是唯一的例外（ADR-0091）。

### 写法：compose 用标准字段，权限里声明

上游的 compose 怎么写，插件就怎么写，Agent 不用学一套新词。规范不再拒绝这些字段，要求权限和字段对得上：只写一边，清单不合法，错误写明缺哪一项。

| compose 字段 | 权限 | 级别 |
| --- | --- | --- |
| `lan.ports`（`x-wonton`，不变，ADR-0019） | `network.lan` | 高 |
| 核心按显卡给（不变，ADR-0018） | `devices.gpu` | 高 |
| `devices: ["/dev/net/tun:/dev/net/tun"]` | `devices.<路径>`，如 `devices./dev/net/tun` | 高 |
| 由主人选的串口，见“设备” | `devices.serial` | 高 |
| 整类 USB 设备，见“设备” | `devices.usb` | 高 |
| 由主人勾的硬盘，见“设备” | `devices.disk` | 很高 |
| `cap_add: [NET_ADMIN]` | `cap.NET_ADMIN`，见“能力” | 很高或最高 |
| `network_mode: host` | `network.host` | 很高 |
| `pid: host`、`ipc: host`、`security_opt` | `host.pid`、`host.ipc`、`security.opt` | 最高 |
| 无（核心按危险模式挂进 sudo 密码文件，ADR-0091） | `host.sudo`，只能和 `privileged`、`host.pid` 一起 | 最高 |
| `privileged: true` | `privileged` | 最高 |
| 卷 `/var/run/docker.sock:/var/run/docker.sock` | `docker.socket` | 最高 |
| 无（只声明权限：核心在插件网络里给一个只读的健康接口，别的插件用它监控大家的健康状态） | `plugins.health` | — |

- 这些都按服务生效，只给写了字段的服务。需要组播的插件可以只让一个小的发现服务用宿主网络，网页服务留在插件网络里。
- `ports`、`networks`、`container_name`、`build` 这些和核心管理冲突的字段照旧拒绝。`ports` 的理由见 ADR-0019。
- 除 `docker.sock` 外，宿主路径的挂载规则不变。`docker.sock` 的源只能写这一个路径，核心换成这台主机上实际的 socket。

### 默认加固

所有插件容器默认带 `no-new-privileges:true`（核心写进 HostConfig 的 `SecurityOpt`），和核心自己的系统容器一致：插件进程不能再经自己的操作拿到新特权（setuid 位、文件能力）。

- `security_opt` 写 `no-new-privileges:false` 退回 Docker 默认，给依赖 setuid 的镜像（容器里用 sudo 的）。这一项不算权限、不要求声明。
- 写 `no-new-privileges` 或 `no-new-privileges:true` 等于默认，同样不要求声明。
- `security_opt` 里的其他写法照上表要 `security.opt` 权限；退回默认和其他写法可以在同一个服务里同用。

**组播和广播不单独设权限，走 `network.host`。** ADR-0019 的实测：桥接网络里组播出不去也进不来，宿主网络里都能通。核心不做组播转发。

### 能力（`cap_add`）

| 级别 | 能力 |
| --- | --- |
| 很高 | `NET_ADMIN`、`NET_RAW`、`NET_BIND_SERVICE`、`SYS_TIME`、`SYS_NICE`、`IPC_LOCK` |
| 最高 | 其余全部，包括 `SYS_ADMIN`、`SYS_MODULE`、`SYS_PTRACE`、`SYS_RAWIO`、`DAC_READ_SEARCH`、`ALL`，以及以后 Linux 新增的 |

判不准的一律算最高。

**宿主网络里的服务默认去掉 `NET_RAW`。** 带着 Docker 默认给的 `NET_RAW`，宿主网络里的容器能抓别的插件的包（ADR-0019 的实测），去掉就抓不到。要用的插件写 `cap.NET_RAW`，比如 Home Assistant 的 DHCP 发现。

**`cap_drop` 不需要权限：它只能减，不能加。** 值和 `cap_add` 一样规范化，`ALL` 也收；和宿主网络默认去掉 `NET_RAW` 合并去重——写了 `NET_RAW` 或 `ALL` 就不再重复加，写别的值时两者都在。

### 设备

四类，按设备在每台主机上是否一样、拔插后编号会不会变来分：

| 类 | 清单 | 授权粒度 | 容器里 |
| --- | --- | --- | --- |
| 路径固定的（`/dev/net/tun`、`/dev/fuse`） | compose 直接写，声明 `devices.<路径>` | 这一个节点 | 同宿主路径 |
| 串口 | `devices.serial` | 主人选的那一个设备 | 清单写的 `target` |
| USB | `devices.usb` | 整类：这台主机上所有 USB 设备 | `/dev/bus/usb` |
| 硬盘 | `devices.disk` | 主人勾的那几块整盘 | 依次 `/dev/sda`、`/dev/sdb`…，NVMe 为 `/dev/nvme0n1`… |

```yaml
x-wonton:
  permissions: [devices.serial, devices.usb, devices.disk]
  devices:
    serial:
      - { service: zigbee2mqtt, target: /dev/zigbee, optional: true }
    usb:
      - { service: scanservjs }
    disk:
      - { service: collector }
```

“设备”一节说的是核心按主机给的设备，宿主路径白名单和 `is_fixed_device` 照旧只放行固定的那几个，清单里不能手写
`/dev/sda`、`/dev/bus/usb`、`/dev/ttyUSB0` 这类路径。

**串口**（Zigbee 棒、ESP 板）：

- 核心从只读挂载的宿主 `/dev`（ADR-0011）列出 `/dev/serial/by-id/*`，安装页显示 by-id 名和它现在指向的 `ttyUSBx`/`ttyACMx`，
  主人给每个 `serial` 条目选一个，记在安装记录里。一个服务只拿到选中的那一个，拿不到旁边的别的串口。
- 建容器时 `PathOnHost` 直接写 by-id 路径，交给 dockerd 解析符号链接（moby 的 `DevicesFromPath` 先解析再取设备号）。
  容器加入节点所属的组，GID 从宿主节点读，不写死 `dialout`（20），同 ADR-0018。
- `optional` 只表示安装时可以不选。选了的设备就是这个服务的一部分：不在时这个服务不启动，同插件的其他服务照常。
- 选具体用哪种协调器（串口还是网络协调器的 `tcp://`）是上游自己的事。Zigbee2MQTT 把串口写成 `optional`，二选一交给它自带的
  首次引导页，核心不为某个插件加语义。

**拔插后自动恢复**：Docker 文档说 `--device` 不能安全地用于会消失的设备，容器不会自己接回。核心盯着选中的 by-id（inotify，读不到事件时
每 2 秒查一次）：

- 消失：这个服务标为异常，类别 `device_unavailable`，详情页写“设备已拔出”。不停容器，上游自己会报错或等待。
- 重现并稳定 2 秒（防 ESP 板复位时反复枚举）：核心自动恢复这个服务，主人不用点。只动拿到这个设备的服务。
- 恢复的方式**待实测**（见“待验证”）：用 by-id 建的容器拔插让 `ttyUSBx` 编号变了以后，`docker restart` 能不能拿到新节点。
  - **restart 能拿到**（dockerd 每次启动都重新生成 OCI 规格、重新解析 `PathOnHost`）：核心只 `restart` 这个服务。容器 ID、
    日志和匿名卷都留着，几秒完成。Docker 自己的重启策略在设备回来以后拉起容器也能拿到新节点，核心和重启策略不冲突。
  - **restart 拿不到**（设备号在建容器时就定死了）：核心按同一份渲染结果强制重建这个服务（摘要没变也重建）。容器 ID 换新、
    匿名卷丢失，插件不能把要留的数据放在匿名卷里（本来就该放数据目录）；Docker 的重启策略拉起的容器还是旧设备号，所以核心
    在设备重现时总是重建，不依赖重启策略。
  - 实测结论和 Docker 版本写进“验证结果”，按结论二选一写死，不两套都留。

**USB**（扫描仪、打印机、SDR 这类经 libusb 用的设备）：`/dev/bus/usb` 下的编号每次插拔都变，逐个选不稳，所以按类给。

- 核心给服务只读、不递归地绑定宿主 `/dev/bus/usb`，加 `device_cgroup_rules: ["c 189:* rwm"]`（189 是 USB 设备的主设备号）。
  只读挂载不妨碍打开设备节点读写（内核对只读挂载只拦普通文件和目录）。新插上的设备在同一个 devtmpfs 里出现，插上即用，
  不用重启。
- 不含 USB 转串口：`ttyUSB`、`ttyACM` 是串口，走 `devices.serial`。
- 提示写明“能访问这台主机上所有 USB 设备”，例：装扫描仪插件，换个 USB 口插扫描仪照样能用，但它也能看到 U 盘读卡器。

**硬盘**（给 Scrutiny 这类读 SMART 的插件）：

- 核心列出 `/dev/disk/by-id` 里的整盘（不含 `-partN`），同一块盘的 `wwn-*` 和型号名合成一项，显示型号、序列号和现在的节点名，
  主人勾选，可以勾几块。
- 装着系统的盘（宿主根目录或 Wonton 数据目录所在的盘）也列出来，标“系统盘”，默认不勾，主人可以勾。认不出哪块是系统盘时
  一块都不标，预览写明认不出。
- 容器里按勾选顺序命名：SATA/SAS 依次 `/dev/sda`、`/dev/sdb`…，NVMe 给命名空间块设备 `/dev/nvme0n1`… 和它的控制器
  `/dev/nvme0`…。宿主那边同样按 by-id 交给 dockerd。
- 核心随 `devices.disk` 给 `SYS_RAWIO`，插件不用也不能为此另写 `cap.SYS_RAWIO`。NVMe 读 SMART 是否还要 `SYS_ADMIN` **待实测**：
  Linux 6.2 起的 `nvme_cmd_allowed` 对以可写方式打开的节点放行 Identify 和 Get Log Page（读代码推断，没测）。实测不要就不给；
  要的话，勾了 NVMe 盘的那次安装按最高级提示和确认（`SYS_ADMIN` 本来就是最高级），只勾 SATA 盘的仍是很高。
- 级别“很高”：带 `SYS_RAWIO` 时，对只读打开的盘也能经 `SG_IO` 发写命令（内核行为，推断，未实测）。提示写明“能读写你勾选的硬盘上的全部数据”。
- 硬盘是固定装着的，不做拔插恢复。盘不在时同串口：服务不启动，类别 `device_unavailable`。

**`/run/udev`**：申请了任一 `devices.*` 权限的服务，核心自动只读挂宿主的 `/run/udev`，不单列权限。Zigbee2MQTT 列适配器、
smartctl 认盘都读 udev 数据库，没有它上游的自动发现用不了。宿主没有 `/run/udev`（没装 udev 的最小系统）时不挂、不算错，预览写明
上游的自动发现可能用不了。核心经部署只读挂进来的宿主 `/run/udev` 判断有没有。

**契约**（新外部接口的检查单）：契约是 Linux 的设备节点和 udev 的 `/dev/serial/by-id`、`/dev/disk/by-id` 命名；提供方是宿主；
真实使用方是 Zigbee2MQTT（串口）、scanservjs（USB）、Scrutiny（硬盘）。契约测试用假的宿主 `/dev` 树（by-id 符号链接加假节点）：
by-id 改指向另一个节点（模拟拔插后编号变化）时，使用方的 compose 和渲染结果不变，核心给的 `PathOnHost`、`GroupAdd`、
cgroup 规则和 `/run/udev` 挂载符合上表；by-id 消失和重现时按“拔插后自动恢复”的结论动了且只动了拿到这个设备的服务。

### 三级提示

每个权限都有一个级别。预览接口把级别和这段话一起返回，App 只负责按级别画；不认识的级别按最高画。

| 级别 | 权限 | 提示 | 确认 |
| --- | --- | --- | --- |
| 高 | `network.lan`、`devices.*`（`devices.disk` 除外） | 列出端口或设备，写明谁能连上来（ADR-0019）；`devices.usb` 写明能访问所有 USB 设备 | 和现有高权限一样，醒目展示 |
| 很高 | `network.host`、`devices.disk`、很高的能力 | 下面的文字，警告色 | 同上 |
| 最高 | `docker.socket`、`privileged`、最高的能力、`host.pid`、`host.ipc`、`security.opt` | 下面的文字，错误色，放在最上面 | 单独确认：点“安装”后再弹一步，要点“我知道这个插件能控制整台主机” |

提示文字由核心给（中文，英文同义），放在安装确认页和确认卡片上：

- **`network.host`：** “这个插件能直接访问你家里的网络和本机所有端口，包括只给本机用的服务，也能绕过 Wonton 的登录连到部分其他插件。它开的端口会出现在本机所有网卡上，包括 Wi-Fi、Tailscale 和 IPv6 地址，本机有公网地址时互联网也能连上。本机开着防火墙时，局域网设备连不进来，要你自己放行。”
- **`devices.disk`：** “这个插件能读写你勾选的硬盘上的全部数据。”
- **`docker.socket`、`privileged` 等最高级：** “这个插件能控制整台主机：读写所有文件和其他插件的数据，改动或停掉 Wonton 自己。这等于把主机的 root 交给它。只装你信任的插件。卸载后，它建的容器和改过的东西不会被清理。”

来源标签（“未审核”，ADR-0031）照常放在旁边。官方索引可以收最高级的插件，审核时看它有没有写清为什么要这项权限。

### 谁拿不到

校验写在清单检查里，安装和 `plugin check` 同一套，错误类别 `invalid_input`。很高和最高级的工具和 Bot 写的插件能用，规则见下一节。

| 情况 | 规则 | 为什么 |
| --- | --- | --- |
| `kind: agent`，或声明了 `matrix.bot`、`bots` | 不能申请很高和最高级 | 第一条不变的规定。宿主网络也不给：Agent 带 shell 工具，一次提示注入就能打到本机只监听 `127.0.0.1` 的服务 |

### Bot 来装、更新和调工具

Bot 碰得到高权限插件，但每一步都由主人亲手点，所以“Agent 和 Bot 拿不到 Docker”仍然成立。主人给 Bot 开了危险模式时，主人私聊里的这些步骤不再逐个点（ADR-0091）。

- **安装：** Bot 发起安装走确认卡片，卡片上写上面的提示（ADR-0033）。最高级在卡片上同样要多点一步。这类卡片不能被“一律允许”放宽：ADR-0024 的放宽只管工具调用，不管安装。写成测试。
- **Bot 写的插件（`dir` 来源，ADR-0033）：** 能申请很高和最高级。这两级每次安装和每次更新都发卡片，不管权限有没有变宽；最高级的卡片上写明“代码会以本机 root 身份运行”。“权限没变宽就不发”只适用于高级和普通权限。
- **最高级插件的工具：** 声明了 `mcp` 也可以。每次工具调用都发一张确认卡片，卡片上写明“这个操作以本机 root 身份执行”。工具标了只读也发。主人不能把它放宽成“一律允许”或“始终允许”，标成只读、放行时段和 ADR-0024 的放宽对这一级都不适用，审计的 `basis` 是 `card`。写成测试。唯一的例外是危险模式（ADR-0091）：开着且消息来自主人私聊时不发卡片，`basis` 是 `danger`；关着时照旧每次发，同样写成测试。很高级插件的工具照常按 ADR-0024（Home Assistant 的工具要给 Bot 用）。

### 宿主网络里的网页怎么接

ADR-0019 实测：宿主开着防火墙时，桥接网络里的核心连不上宿主网络里的服务。不改防火墙，App 就打不开 Home Assistant。

**核心给有 `webui` 或 `mcp` 的宿主网络服务配一个中继：** 一个小容器和服务一样在宿主网络里，监听插件数据目录里的 unix 套接字，转发到 `127.0.0.1:<端口>`。网关连这个套接字。unix 套接字不走防火墙，主人不用改任何规则（见“验证结果”）。

- `relay-webui`、`relay-mcp` 是核心中继的保留服务名，插件不能声明，包括普通网络的服务；否则生成的容器名和 `wonton.service` 标签会与中继重名。
- 应用可以只监听 `127.0.0.1`，这样局域网绕不过网关。这由插件自己配，能配的官方插件都要配。
- `sso: proxy` 不能和宿主网络里的 `webui` 一起用，清单不合法：应用要监听局域网才能被设备发现时，局域网里的任何设备都能伪造身份头，冒充主人。用 `oidc` 或 `none`。
- 宿主网络里的服务出网不经 ADR-0023 的出口容器，看的是宿主自己的路由。预览如实写明，不用再声明 `network.egress`。
- 端口被占时，应用可能只记一行日志、容器照样运行（ADR-0019）。所以宿主网络里的 `webui` 服务要写 `healthcheck`，装好后还要经中继探测网页。
- 安装时核心顺便试一次不经中继直连。连不上时再从中继容器读监听表：端口只监听 `127.0.0.1`（或 `::1`）的，应用本来就绕不过网关，不提示；还监听其他地址的，才在详情页提示“本机防火墙可能挡住了外来连接，局域网设备可能连不进来”。读不到监听表时只记 warn，不提示。

### 接口

`wonton-protocol` 的权限项加 `risk`（`high`、`very_high`、`highest`，不认识的落到 `Other`）和 `notice`。预览加可选的设备列表，安装请求加设备选择。**动接口，先合。**

预览的设备项（`PluginDevice`）带类（`serial`、`disk`，不认识的落到 `Other`，App 显示“要更新 App”并不能提交）和“系统盘”标记；
`devices.usb` 没有可选项，只在权限行里出现。详情页的设备状态用类别 `device_unavailable`。

## 理由

- 把判断交给主人，和 ADR-0011 的“安全只做必要的”、ADR-0033 的“这是主人的选择，提示风险，不在底层拦”一致。
- 写标准 compose 字段，Agent 照着上游文档写就对，符合“能一次写对”。ADR-0019 拒绝 `ports` 是因为同一行字在 Wonton 里语义不同；`network_mode: host`、`cap_add` 这些语义完全相同，没有这个问题。
- Agent 和 Bot 的那条线守在清单校验上，一处判定，好测。

## 没选的做法

- **ADR-0019 留的“宿主网络只给发现用的小服务，不许有网页”。** 能减小暴露，但 Home Assistant 这类应用的发现和网页在同一个进程里，拆不开，等于还是用不了。按服务生效已经允许作者这样做，不强制。
- **核心做组播转发或 avahi 反射。** 要自己维护一套发现中继，mDNS、SSDP、WS-Discovery 各一种，还要把系统 D-Bus 挂进核心（ADR-0019）。宿主网络一步到位。
- **macvlan。** 理由见 ADR-0019。
- **允许清单写任意宿主路径。** 固定路径和用户的磁盘布局对不上，真正的需求（媒体在别的硬盘上）应该让共享文件夹能指到任意目录（ADR-0010）。要全盘访问的备份类插件用最高级的权限。
- **`cap_add` 白名单，名单外拒绝。** 又是“声明了也不给”，会让下一个应用用不了。名单外算最高级。
- **按插件全局给能力。** Compose 本来按服务写，全局给只会多给。
- **设开关，默认不让装最高级。** 主人要先找到开关才能装 Portainer，多一步也不多一分安全，提示和单独确认已经够了。
- **串口按类放行全部串口**（HA 的 `uart`）：插上即用，但 Zigbee2MQTT 也能碰旁边的 ESP 板。串口的 by-id 稳定，按设备给做得到。
- **USB 逐个选**：`/dev/bus/usb` 的编号每次插拔都变，选中的那个拔插一次就失效。
- **在容器外补 cgroup 规则**（HA 经 OS Agent）**或装宿主 udev 规则**（IOTstack）：前者要宿主上有我们的代理，后者改宿主配置，
  都违反“只要 Linux + Docker”。
- **`privileged` 加 `/run/udev`，选设备交给应用**（Umbrel 的 Zigbee2MQTT）：等于为一根 Zigbee 棒交出整台主机。
- **自由文本框填设备路径**（Unraid、Runtipi、CasaOS）：填 `/dev/ttyUSB0` 会漂移，也不校验。
- **系统盘一律不让勾**（HA）：Scrutiny 看不到系统盘的健康，而系统盘坏了恰恰最要紧。标出来、默认不勾已经足够提醒。

## 代价与风险

- **高权限插件可以绕过隔离。** 宿主网络里的插件连得上本机只监听 `127.0.0.1` 的服务，也连得上声明了 `network.egress` 的插件，`sso: proxy` 的那些因此可能被它冒充主人（ADR-0019 的实测）。最高级的插件能做任何事。这是主人确认过的取舍，靠提示说清，核心不再试图拦。
- **主人可能不看提示就点。** 只有最高级多一步。
- **共用一台主机的几套核心**（开发环境）：宿主网络的端口会互相冲突，开发环境里后装的装不上，错误要写清。
- **卸载清不干净。** 插件经 Docker socket 建的容器不带 `wonton.plugin` 标签，核心删不到。提示里已写明。
- **容器逃逸类的问题不再是 Wonton 的边界。** 用户给了最高级就等于给了 root。
- **USB 按类给就是整类都给。** 装扫描仪插件，它也能读 U 盘、读卡器里的数据。提示写明。
- **拔插恢复会重启服务。** 设备回来时服务被重启或重建，正在进行的操作（如 ESPHome 刷机）会断。

## 已定

主人 2026-11-04 定：

1. 宿主网络放开，声明加醒目提示，Bot 来装走确认卡片。
2. 整体重定：能让插件用不了的限制都放开，改成声明加醒目提示。Docker socket 和特权模式放最高一级，单独确认，提示写得最重。
3. Agent 和 Bot 插件拿不到 Docker socket、特权模式和宿主 shell。Bot 装任何高权限插件都走确认卡片。没声明的能力照旧不给。（“宿主 shell”和“都走确认卡片”已被 ADR-0091 改写，见“两条不变”。）

主人 2026-11-12 定：

4. 串口按主人选的那一个设备授权，不按类放行全部串口。
5. 串口拔掉再插回由核心自动恢复；restart 还是重建先实测再定。
6. USB 改按类授权。
7. 新增硬盘一类，级别“很高”；系统盘能勾，默认不勾、标“系统盘”。
8. `/run/udev` 只读随 `devices.*` 自动给，不单列权限。
9. Zigbee2MQTT 的串口和网络协调器二选一交给它自带的引导页，核心不管。
10. ESPHome 刷写先按单设备授权实测（见“设备待验证”）；实测确实不行，才另加“全部串口”权限，级别比单设备的高（保留为实测项）。

协调会话按这几条定了细则（2026-11-04）：Agent 和 Bot 插件也不给宿主网络；最高级插件的工具每次调用都发卡片，不能放宽；Bot 写的插件能申请最高级，很高和最高级每次安装和更新都发卡片。

## 验证结果（2026-11-04）

开发机，Docker Engine 29.7.2，开着 ufw 和 ufw-docker。镜像 `alpine/socat:1.8.0.3`。一个宿主网络的容器监听 18124 端口，另一个宿主网络的容器做中继：`UNIX-LISTEN:/s/web.sock → TCP:127.0.0.1:18124`。

| 从哪连 | 结果 |
| --- | --- |
| 宿主，`127.0.0.1:18124` | 通 |
| 宿主，局域网地址:18124 | 通 |
| 桥接网络的容器，经 `host-gateway` 直连 | 超时（被 ufw 挡住，和 ADR-0019 一致） |
| 桥接网络的容器，挂同一个目录，连 unix 套接字 | 通 |

宿主网络、组播和抓包的实测见 ADR-0019。

### 中继与协议升级（2026-11-05）

开发机，Docker Engine 29.7.2，`systemctl is-active ufw` 为 active，没改防火墙规则。核心按本 ADR 给一个宿主网络的网页
服务配了 `alpine/socat:1.8.0.3` 中继（同一网络模式，挂插件数据目录，监听 unix 套接字转发到 `127.0.0.1:<端口>`）。
直连超时和防火墙提示另用同机的纯网页样例测过，条件和这里相同。

| 检查 | 结果 |
| --- | --- |
| 装好后中继和网页容器都在宿主网络，套接字文件存在 | 通 |
| 从另一个容器经 unix 套接字请求网页 | 200 |
| 从核心网络经宿主网桥直连网页，3 秒时限 | 连接超时（被 ufw 挡住） |
| 详情接口的防火墙提示 | 有 |
| 登录核心、经网关打开插件页（网关 → 套接字 → 中继 → 网页） | 200 |
| 经同一路径做 WebSocket 升级，发一帧收回同一帧 | 101，帧原样回来 |
| `docker kill` 中继后在原进程里重启（套接字文件残留） | 没有 `unlink-early` 时 socat 报 “exists”，容器反复重启；加上 `unlink-early` 后启动成功，套接字继续可用 |

### 只监听回环时的防火墙提示（2026-11-05）

开发机，Docker Engine 29.7.2。假 Docker 下让行为测试复现后修复：只监听回环、监听所有地址、读不到监听表三种情况。另用 `public.ecr.aws/docker/library/busybox:1.37` 在宿主网络里 `nc -l -s 127.0.0.1`，确认真实内核的 `/proc/net/tcp` 格式能被解析成回环监听。

### 最高级工具的卡片（2026-11-05）

假插件和假 Docker 的行为测试：最高级的只读工具连续调用两次，每次一张卡片，卡片都写明以 root 身份执行；同时打开只读标记、始终允许和一律允许后仍然发卡，审计 `basis` 为 `card`；拒绝卡片时工具没有执行。

### Home Assistant 作为第一个样例（2026-11-06）

开发机，开着 ufw。插件 `wonton-plugin-home-assistant` 头部 `a9444526`，Home Assistant 2026.10.4，核心 main `d3f3bf96`（手动流程）和 `wonton-core:m1v4b`（冒烟）。主服务 `network_mode: host`，只加 `cap.NET_RAW`，`privileged=false`。

| 检查 | 结果 |
| --- | --- |
| 自动发现局域网设备（`config_entries/flow/progress`） | 发现 apple_tv、ipp、nut、upnp 四类 |
| 手机尺寸的浏览器经网关打开，添加一个 zeroconf 发现的 IPP 集成 | 成功 |
| 重装后这个集成 | 保留，`entry_id` 和发现来源都不变 |
| 网页只监听 `127.0.0.1`，经中继打开；匿名请求 | 打得开；匿名 401 |
| HA 不接受核心中继带来的转发头 | 实测带转发头直连返回 400；插件用官方 nginx 去掉转发头后正常 |
| 主人身份（官方回环可信网络登录，经网关 WebSocket 查当前用户） | 认出主人，是管理员 |

自动发现和添加集成不需要 `privileged`，`NET_RAW` 只为 DHCP 发现。没有做 `privileged` 对照，也没有测蓝牙和 USB。

蓝牙走不通：完整接入还要挂 `/run/dbus`，清单校验拒绝这个宿主路径。

**待验证：** 在 App 里打开 Home Assistant（上面用的是手机尺寸的浏览器）。没有防火墙的主机。蓝牙。

### 设备待验证

- **拔插后 restart 能不能拿到新节点**（决定“拔插后自动恢复”二选一）：用 by-id 的 `PathOnHost` 建容器；拔掉，先插另一个 USB 串口
  占住原来的 `ttyUSB0`，再插回原设备让它变成 `ttyUSB1`；`docker restart` 后在容器里看 `target` 的设备号是不是新的、能不能打开。
  同一台机器再测 Docker 重启策略拉起的情况。写明 Docker Engine 版本和 cgroup 版本。
- 核心在容器里能不能直接看到宿主 `/dev/serial/by-id`、`/dev/disk/by-id` 的符号链接（推断能，没测）。
- ESP32-S3/C3 这类原生 USB 的板子刷机复位会重新枚举，按设备授权时 ESPHome 刷机能不能成功；不行才另加“全部串口”权限（“已定”第 10 条）。
- NVMe 读 SMART 要不要 `SYS_ADMIN`；带 `SYS_RAWIO` 时只读打开的盘能不能经 `SG_IO` 写。

## 关联

- [ADR-0006](0006-plugin-system.md)、[ADR-0011](0011-security-model.md)、[ADR-0018](0018-gpu-for-plugins.md)、[ADR-0019](0019-lan-services-for-plugins.md)、[ADR-0023](0023-egress-proxy.md)、[ADR-0024](0024-confirmation-scope.md)、[ADR-0031](0031-plugin-sources.md)、[ADR-0033](0033-plugin-tabs.md)
