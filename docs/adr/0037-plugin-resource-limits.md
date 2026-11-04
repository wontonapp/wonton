# ADR-0037 插件的资源限制：闲时不限速，忙时让路，内存和进程数有上限

- 状态：已接受（2026-11-05）

## 背景

插件和 Agent 的容器现在什么限制都没有。主人 2026-11-05 提出：最严重的情况是某个插件或 Agent 占满 CPU，远程 SSH 连不上，整台机器卡死；Linux 自带这种能力，常规做法照搬就行，别把限制做得太死。

机器卡死的原因不只 CPU：

- **CPU：** 内核按权重分。容器默认和 `sshd` 权重一样，都在 `system.slice` 里，几十个忙碌的进程抢的是同一份。
- **内存：** 这是更常见的死法。内存快满时内核反复回收、换页，所有进程都卡在磁盘上，SSH 也进不来，OOM 杀手要很久才动手。
- **进程数：** Agent 跑出的 fork 炸弹会耗尽整台机器的进程表。

## 别人怎么做

- **Kubernetes：** 所有 Pod 放进父 cgroup `kubepods.slice`，给系统留一份（`system-reserved`），父 cgroup 的内存上限是总量减去这一份。CPU 按权重分，闲时不限速；内存超限由 OOM 杀手处理。这是本篇抄的主要做法。
- **CasaOS：** 每个应用写 `cpu_shares` 和 `mem_limit`，没有父 cgroup，多个应用加起来仍然能占满机器。
- **Docker 文档：** 建议用 `--memory`、`--pids-limit`、`--cpu-shares` 和 `--cgroup-parent`，都是标准做法，不用自研。

Kubernetes 和 CasaOS 这两条凭对它们的了解写，没有逐条再查。

## 决策

### 两层限制

| 层 | 管什么 | 由谁设 | 哪些主机有 |
| --- | --- | --- | --- |
| 每个服务 | 权重、内存上限、进程数、OOM 优先级 | 核心建容器时经 Docker API 设 | 所有主机 |
| 所有插件的总量 | 总权重、总内存，给系统留一份 | 安装脚本写一个 systemd slice | systemd 加 cgroup v2 |

### 每个服务的默认值

| 字段 | 默认值 | 为什么 |
| --- | --- | --- |
| `CpuShares` | 512 | 只是权重，闲时不限速；比没写权重的容器（如 `sshd`、核心）低一档。 |
| `Memory` | 主机总内存的 80%，且不低于 `requirements.memory` | 普通插件碰不到；单个服务失控时不会吃光其他插件的份额。读不出总内存时不设，不猜。 |
| `MemorySwap` | 等于 `Memory`，即不用交换区 | 超限时直接杀，不把机器拖进换页。只写 `Memory` 时 Docker 会再给同样多的交换区。 |
| `PidsLimit` | 4096 | 拦住 fork 炸弹，64 核主机上的普通应用也够用。 |
| `OomScoreAdj` | 500 | 全局内存不够时先杀插件，不先杀 `sshd` 和核心。 |

默认不设 CPU 硬上限（`NanoCpus`）：闲着的核不该白白浪费。

Agent 插件没有例外。一次性服务也一样。核心自己、Tuwunel 和其他系统服务不受这些限制，继续用默认权重。

### 所有插件的总量：`wonton-plugins.slice`

安装脚本以 root 运行，写入 `/etc/systemd/system/wonton-plugins.slice`：

```ini
[Slice]
CPUWeight=50
IOWeight=50
# 给系统留 max(1G, 总内存的 10%)，最多留总内存的 25%。
MemoryMax=<总内存 - 留出的>
MemoryHigh=<MemoryMax 的 90%>
TasksMax=50%
```

- 写好后在 `.env` 里记 `WONTON_PLUGIN_SLICE=wonton-plugins.slice`，核心给每个插件容器设 `CgroupParent`。没有这个变量时不设，只有每个服务那一层。
- `MemoryHigh` 是软上限：超过时内核先回收、放慢这些进程，不杀。`MemoryMax` 才杀。
- 核心和系统服务不在这个 slice 里，留出的内存和 CPU 权重就是给它们和 `sshd` 的。
- 不设总量的 CPU 硬上限（`CPUQuota`）。权重在忙时已经让路，验收时实测 SSH 和核心的响应；实测不够再加。

### 哪些主机上是什么效果

| 主机 | 效果 |
| --- | --- |
| systemd 加 cgroup v2 | 两层都有。 |
| cgroup v1，或 `cgroupfs` 驱动（没有 systemd） | 只有每个服务那一层，cgroup v1 也支持这些字段。核心没有权限写父 cgroup，安装脚本也不写：systemd 不管的路径没人负责重启后恢复。 |
| rootless Docker | 安装脚本不支持。手工部署时，cgroup 没有委托给用户的话 Docker 会丢弃限制并在建容器的响应里给 `Warnings`，核心写进日志。 |
| 手工部署，没跑安装脚本 | 只有每个服务那一层。 |

安装脚本只在 `docker info` 报告 `CgroupDriver=systemd`、`CgroupVersion=2` 时写 slice，否则打印一行说明只有每个服务的限制。`wonton-core plugin host` 同时报告 Docker 是否支持这些限制（`MemoryLimit`、`SwapLimit`、`CPUShares`、`PidsLimit`）和 `WONTON_PLUGIN_SLICE`。

### 插件怎么声明：compose 的标准字段

不加 `x-wonton` 字段，沿用 ADR-0036 的原则：上游的 compose 怎么写，插件就怎么写。服务上开放 `mem_limit`、`mem_reservation`、`cpus`、`cpu_shares`、`pids_limit`。

- 写了就照写的给，没写的用上面的默认值。
- 不设上下限。权限不高，总量又有 slice 兜着，和 ADR-0036 “声明了就给”一致。
- `mem_limit` 写了时 `MemorySwap` 仍等于它，插件不能要交换区。

### 主人调整

- 详情页有“内存上限”和“CPU 上限”两项，对这个插件的每个服务生效，覆盖 compose 和默认值；可以调高，也可以恢复默认。
- CPU 上限默认是不限。
- 接口：`PUT /_wonton/api/plugins/{id}/limits`，内容是 `{"memory": 字节数或 null, "cpu_percent": 百分比或 null}`，`150` 表示 1.5 个核，`null` 恢复默认。
- 用 `POST /containers/{id}/update` 当场生效，不重启容器。写进插件的安装记录，重建和升级后保留。
- 调低到比当前用量还小时，内核先回收，回收不了就杀，App 按下面的方式如实显示。

### 超过限制时用户看到什么

- **内存：** 核心订阅 Docker 的 `oom` 事件，记下哪个插件的哪个服务、什么时候。
  - 容器起不来、在反复重启：插件的状态是异常，原因写“服务 web 内存不够，被系统结束（上限 4 GB）”。
  - 只杀了子进程，或者重启后好了：插件照常运行，`notices` 里留一行，24 小时后消失。
  - 事件只存在内存里，核心重启后丢失。核心启动时还会读容器的 `State.OOMKilled`，反复重启的情况不会漏。
- **进程数：** 超限时 `fork` 返回错误，由插件自己报，核心看不到，也不伪装成知道。
- **CPU：** 只是变慢，不算异常，不提示。

### 不管的

- **磁盘写满：** cgroup 管不了磁盘用量。Docker 的 `--storage-opt size` 只支持部分文件系统。1k 那次事故要另外处理，不在这篇里。
- **显存：** Docker 和 cgroup 没有这项限制。

## 实测

Docker 29.7.2，Linux 7.2.5-3-omarchy，cgroup v2，systemd 驱动：

| 操作 | 结果 |
| --- | --- |
| `--cpu-shares 512` | `cpu.weight=59`，1024 对应 100，不写也是 100 |
| `-m 64m` 不写 swap | `memory.max=64M`，`memory.swap.max=64M` |
| `-m 64m --memory-swap 64m` | `memory.swap.max=0` |
| `--memory-reservation 32m` | `memory.low=32M`，`memory.high` 不变 |
| `--cgroup-parent wonton-limtest.slice`，slice 不存在 | 容器正常起来，systemd 自动建出 `wonton.slice/wonton-limtest.slice`，不需要 root，没有任何限制 |
| 子进程超内存被杀，主进程活着 | 发 `oom` 事件；主进程退出前读不到别的信号，退出后 `OOMKilled=true`，退出码是主进程自己的 0 |
| 主进程超内存被杀，`unless-stopped` | 每次都发 `oom` 事件，状态 `restarting`，`OOMKilled=true`，退出码 137 |

父 slice 超限时 `oom` 事件是否还发，以及压满 CPU 和内存时 SSH 和核心的响应，由验收实测，结果写回这里。

## 没选的做法

- **默认设 CPU 硬上限：** 闲时也限速，白白浪费。主人的要求是普通插件不受影响。
- **只有每个服务一层：** CasaOS 的做法，多个插件加起来仍然能占满内存。
- **核心自己写父 cgroup 或调 systemd：** 核心在容器里，cgroup namespace 里看不到父级。要么挂 D-Bus，要么起特权容器，都比安装脚本写一个文件复杂，而且绕开了 systemd。
- **每个插件一个子 slice，按插件分总量：** 要为每个插件写 unit，核心没有权限；按服务设已经够用。
- **内存只用软上限：** `MemoryHigh` 不杀进程，只放慢。失控的进程能把自己和机器一起拖住，需要有硬上限收场。
- **清单里新加 `resources` 字段：** 和 compose 已有的字段重复，Agent 还得多学一套。

## 代价与风险

- 80% 和 4096 是经验值。内存特别大的单体应用可能碰到，由插件写 `mem_limit`、`pids_limit`，或者主人调高。
- `IOWeight` 只有在 BFQ 或 iocost 生效时才起作用，常见的 NVMe 默认调度器下没有效果，I/O 抢占没有解决。主要靠内存上限避免换页引发的 I/O 风暴。
- 共用内存的显卡（ADR-0018）跑模型时显存算在内存里，会碰到内存上限。
- 单个服务的默认上限按建容器时的总内存算，换内存条后要重建才更新。
- 一键升级核心不跑安装脚本，已有主机要重跑一次才有 slice。绿地阶段接受。
