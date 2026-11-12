# ADR-0099 按主机显卡厂商选插件镜像：同一插件，NVIDIA 用 CUDA 版，AMD 用 ROCm 版

- 状态：提议（2026-11-12）。方向由主人定（11-12：底座按主机显卡厂商选镜像，只用上游官方发的各厂商镜像，上游没有的不自建）；
  标“待定”的几处等主人定。补充 [ADR-0018](0018-gpu-for-plugins.md)：那篇定“给容器什么设备”，这篇定“给容器什么镜像”，
  设备的给法、探测和失败原因不变

## 背景

ADR-0018 让一份清单在 AMD 和 NVIDIA 的主机上都能拿到显卡，但镜像只有一个。跑模型的上游大多按厂商发不同的镜像，同一个
镜像换到另一家的显卡上，要么用不上加速，要么起不来。主人的两台 AMD 服务器都是 Strix Halo 核显，现有插件在上面只能用 CPU
或 Vulkan。

上游怎么发（2026-11-12 查 Docker Hub、GHCR 的标签和各项目的 Docker 文档）：

| 上游 | NVIDIA | AMD | 备注 |
| --- | --- | --- | --- |
| Ollama | `ollama/ollama:<版本>`（同一个镜像也带 CPU 和 Vulkan） | `ollama/ollama:<版本>-rocm` | 0.40.16：默认镜像有 amd64、arm64，`-rocm` 只有 amd64。默认镜像经 Vulkan 也能用 AMD 显卡，文档说 Vulkan 读可用显存要 root 或 `CAP_PERFMON`，否则按估算调度。`-rocm` 用 ROCm 7.2.1，文档的 Linux 支持表列着 gfx1151 |
| llama.cpp | `ghcr.io/ggml-org/llama.cpp:server-cuda`（另有 `-cuda13`） | `:server-rocm` | 还有 `-vulkan`、`-intel`（SYCL）、`-musa`。ROCm、Intel 版只有 amd64。文档说 GPU 镜像在 CI 里只编译、不测试 |
| vLLM | `vllm/vllm-openai:<版本>` | `vllm/vllm-openai-rocm:<版本>-rocm100` | **仓库名不同**，不只是标签后缀。AMD 自己另发 `rocm/vllm`，按 `rdna`、`cdna` 分标签 |
| ComfyUI | 上游（Comfy-Org）不发镜像 | 上游不发；AMD 发 `rocm/comfyui:comfyui-0.18.2.amd0_rocm7.2.0_ubuntu24.04` | AMD 的镜像约 23 GB、只有 amd64，文档列的支持硬件是 MI300X 这类数据中心卡，示例命令带 `SYS_PTRACE`、`seccomp=unconfined`、`ipc=host` |

由此得出三条约束：厂商之间可能是不同的仓库，不能只拼标签；厂商镜像支持的架构常比默认镜像少；ComfyUI 没有“上游官方”的
镜像，只有显卡厂商发的。

Strix Halo（gfx1151）对 ROCm 的支持（2026-11-12 查 AMD 文档）：Radeon 和 Ryzen 的 Linux 支持表从 ROCm 7.2 起列 gfx1150、
gfx1151，7.2.1 的表列着 Ryzen AI Max+ 395，系统只列 Ubuntu 24.04.4；ROCm Core SDK 10.1.0 的发行说明把 Ryzen AI Max 300 系列
列为 gfx1151（RDNA 3.5），并加了 Ubuntu 26.04.1（内核 7.0）。所以镜像里的 ROCm 不低于 7.2 时用不着 `HSA_OVERRIDE_GFX_VERSION`。
ROCm 太旧的镜像认不出这块显卡，ADR-0018 实测过：ROCm 5.7 的 rocminfo 报“unrecognized id”，健康检查没过，安装回滚。

主人的两台 AMD 服务器（2026-11-12 只读查看）：

| | 甲 | 乙 |
| --- | --- | --- |
| 显卡 | Strix Halo `[1002:1586]`，KFD 报 `gfx_target_version 110501`（gfx1151） | 同左 |
| 系统、内核 | Ubuntu 26.04，7.0.0 | Ubuntu 26.04，7.0.0 |
| Docker | 29.7.2，只有 `runc` | 29.9.3，只有 `runc` |
| 设备节点 | `/dev/kfd`、`renderD128` 属 `render`，`card*` 属 `video`，都是 0660 | 同左 |
| 划给核显的显存 | 512 MiB（其余靠 GTT，约 124 GiB） | 64 GiB |

和 ADR-0018 验证用的开发机不同，这两台的 `/dev/kfd` 不是 0666，非 root 的容器必须加入 `render` 组才打得开。ADR-0018 按宿主
节点的 GID 加组，正好覆盖这种情况。

## 决策

### 清单：只多一张“哪家换什么镜像”的表，其余照 Compose

compose 里服务的 `image` 照常写，是**通用镜像**：没有专门版本的厂商用它，不给显卡时（`optional` 而本机没有，或可选权限关着）
也用它。按厂商换的镜像写在 `x-wonton.gpu.images` 里，只换 `image` 这一个字段：

```yaml
services:
  ollama:
    image: ollama/ollama:0.40.16@sha256:910d…        # NVIDIA（CUDA），不给显卡时跑 CPU
x-wonton:
  permissions: [devices.gpu]
  gpu:
    services: [ollama]
    images:
      ollama:
        amd: ollama/ollama:0.40.16-rocm@sha256:85b3…
  requirements:
    gpu: { need: optional, vendors: [nvidia, amd] }
```

- `gpu.images` 的键是 `gpu.services` 里的服务；下一层的键是厂商（`amd`、`nvidia`、`intel`，和 `requirements.gpu.vendors` 同一套），
  值是完整的镜像引用，可以换仓库（vLLM 就要换）。
- **写了 `gpu.images` 就必须写 `requirements.gpu.vendors`**，`images` 里的厂商都要在里面。`vendors` 说的是“这个插件在哪几家的显卡
  上能用”，`images` 只说“其中哪几家换镜像”。不写 `vendors` 等于哪家都行，Intel 主机上就会拿通用镜像去配 Intel 的设备，这是
  作者没想过的组合。`vendors` 里有、`images` 里没有的厂商用通用镜像，这是作者明说了的。
- 不给显卡时一律用通用镜像。`need: optional` 本来就要求插件自己退回 CPU，所以它的通用镜像得能在 CPU 上跑；只有厂商专门版、
  没有能跑 CPU 的镜像的插件写 `need: required`。没有匹配的厂商就是“本机显卡不支持”，不静默退回 CPU。
- 换镜像不是权限：主人确认的仍是 `devices.gpu`，ADR-0033 的“换镜像不算权限变宽”照旧。

### 核心：自己探测厂商，不让用户填

- 厂商用 ADR-0018 的探测结果：sysfs 里渲染节点的 PCI 厂商号（AMD `0x1002`、Intel `0x8086`），加上 `nvidia` 内核模块和 Docker
  的 `nvidia` 运行时。只算**能用的**：有 NVIDIA 卡但没有运行时，这家不算，原因照 ADR-0018 写。不看 `rocminfo`、`nvidia-smi`。
- **选哪家**：按 `requirements.gpu.vendors` 的书写顺序，取本机第一家能用的，`vendors` 从此是有序的列表。一家都没有：`required`
  装不了（`gpu_unavailable`，原因写成“本机的显卡（AMD）不在这个插件支持的 NVIDIA 里”），`optional` 用通用镜像、不给设备，
  预览写明。两家都有的主机只给选中那家的设备，和 ADR-0018 的“限定了厂商时只给那家的”一致；AMD 和 Intel 混装时设备仍然分不开
  （都在 `/dev/dri`），只是镜像按选中的那家。**待定 1**：主人能不能在预览里换一家。
- **镜像的架构也要对**：选中那家的镜像不支持本机架构时，这家算不能用，接着看下一家，都不行就按上一条处理。核心在预览时经
  Docker 的 `GET /distribution/{name}/json` 读镜像的平台列表（不拉镜像，核心升级已经这样读过）。读不到（离线、仓库拒绝）时如实
  显示查不出来，不拦安装，拉不下来照常报 `image_pull_failed`。`requirements.arch` 仍管整个插件，只对通用镜像。
- 核心给拿到显卡的服务注入 `WONTON_GPU_VENDOR`（`amd`、`nvidia`、`intel`），给了显卡才定义，和 `WONTON_PERMISSION_*` 一样不定义
  而不是空串。各家的命令行参数或环境变量不同时，插件用 `${WONTON_GPU_VENDOR}` 插值，不用为每家各写一份 `environment`。
- 安装记录存选中的厂商。重建、重启、核心升级都用记录里的那家，主机换了显卡也不悄悄换镜像；只在安装、插件升级和打开、关上
  可选的 `devices.gpu` 时重新选（会换镜像，按 ADR-0057 只重建渲染结果变了的服务）。装好后显卡拔了，照 ADR-0018 由容器自己报错。
- 预览和确认卡片的显卡那一行加上选中的厂商和镜像，如“AMD 显卡：/dev/kfd、/dev/dri，加入组 …；用 ROCm 版镜像
  `ollama/ollama:0.40.16-rocm`”。各家镜像大小差得多（AMD 的 ComfyUI 约 23 GB），读平台列表时拿得到压缩后的大小就一并写出。

### 安全：ROCm 不多给

- 设备和组照 ADR-0018：`/dev/kfd`、`/dev/dri`，加入宿主节点的 GID。`no-new-privileges`、作者写的 `cap_drop`（含 `ALL`）、非 root 的
  `user` 都照旧。ROCm 做计算不需要额外的能力：ADR-0018 在 gfx1151 上以 `1000:1000`、不加任何能力跑通了 `rocminfo`。
- 上游文档常写的 `--cap-add SYS_PTRACE`、`--security-opt seccomp=unconfined`、`--ipc=host`（AMD 的 ComfyUI、vLLM 示例），核心不因为
  是 AMD 镜像就给。要的插件照 ADR-0036 各自申请，主人看得见。共享内存由插件写 `shm_size`。
- 核心不设 `HSA_OVERRIDE_GFX_VERSION` 这类按显卡型号的变量：那是镜像和显卡怎么搭配的事，核心不为某个镜像特判（ADR-0015）。
  真要设的插件自己写进 `environment`。

### 校验：`plugin check` 和官方索引

- `plugin check`：`gpu.images` 的服务在 `gpu.services` 里，厂商在 `requirements.gpu.vendors` 里，写了 `images` 就有 `vendors`，
  厂商版镜像和通用镜像不相同。字段进规范的下一版，`spec_floor` 加一行。
- `plugin check --official` 和 `check-index`：厂商版镜像和通用镜像一样要按摘要固定；读每个镜像的平台列表，比 `requirements.arch`
  少时给提醒，不算错误（上一节“镜像的架构也要对”兜住了），读不到算不合格（官方索引收录时要联网）。
- “只用上游官方镜像，上游没有的不自建”是官方索引的收录规则，写进商店仓库的审核清单，由人审；核心不判断镜像是不是官方的，
  社区插件照旧可以用任何镜像。**待定 2**：显卡厂商发的镜像（`rocm/comfyui`、`rocm/vllm`）算不算“上游官方”。

## 没选的做法

- **Compose 的 `profiles` 或多份覆盖文件（`compose.amd.yml`）**：最像 Compose，但核心的插件包是一份 compose，覆盖文件能改任何字段，
  主人确认的内容会随主机变，校验也要对每种组合各跑一遍。只换 `image` 时，一张表就够了。
- **在镜像引用里插值（`image: ollama/ollama:${WONTON_GPU_TAG}`）**：按摘要固定后标签不起作用，每家的摘要不同；vLLM 换的是仓库名。
- **厂商版里也能覆盖 `environment`、`command`**：先不放，用 `WONTON_GPU_VENDOR` 插值；有插件真用得上时再加，加字段比删字段容易。
- **核心自建 ROCm 镜像或给上游补**：主人定了不自建；改上游、发修改版也违反 AGENTS.md 的“不改上游源码”。
- **AMD 用 Vulkan 的通用镜像代替 ROCm**：Ollama、llama.cpp 都能这样跑，不用这篇的机制；但 PyTorch 系（ComfyUI、vLLM）没有 Vulkan
  这条路。作者觉得 Vulkan 够用时，`vendors` 写上 `amd`、`images` 里不写它，就是这条路，两种做法并存。
- **AMD Container Toolkit（`amd-ctk`，CDI `amd.com/gpu=all`）**：要主人在宿主装包、改 daemon 或生成 CDI 规范，文档只写 Instinct 卡和
  Ubuntu 22.04、24.04；`--device` 什么都不用装，ADR-0018 已在 gfx1151 上跑通。
- **让主人选“我的显卡是哪家”**：探测已经准确，多一个选项只会填错。允不允许改，见待定 1。
- **按显卡架构细分（`rdna`、`cdna`、`gfx1151`）**：`rocm/vllm` 这样分了，但家用主机几乎都是 RDNA，先只分到厂商；要细分时在厂商
  下面再加一层，见代价。

## 代价与待验证项

- 只分到厂商：同是 AMD，数据中心卡和核显要不同镜像时分不开，作者只能挑一种（家用挑 RDNA 的）。
- `vendors` 从无序的集合变成有序的列表：已有清单的顺序从此有意义，写反了会选到作者不想要的那家。
- 预览多一次读镜像平台的网络请求，离线时这一项查不出来。
- 厂商版镜像比通用镜像大得多，磁盘提示要跟上（AMD 的 ComfyUI 约 23 GB）。
- 记录里的厂商和主机实际的显卡对不上时（换了显卡）不自动换，要主人重装或升级。

待验证（实现之后在主人的一台 Strix Halo 服务器上做，另开卡）：

1. Ollama `-rocm`：以非 root、`cap_drop: [ALL]`、只加宿主 `render`、`video` 的 GID 起来，`ollama ps` 显示模型在 GPU 上，`rocminfo`
   认出 gfx1151，不设 `HSA_OVERRIDE_GFX_VERSION`。同一台上拿通用镜像（Vulkan）对比一次速度。
2. 同一插件在没有显卡的主机上（或可选权限关着）用通用镜像跑 CPU；`required`、`vendors: [nvidia]` 的插件在只有 AMD 的主机上，
   预览的 `blocked` 写清原因。
3. 只划了 512 MiB 显存的那台：`requirements.gpu.memory` 查不出来时如实显示、不拦安装，模型实际能用 GTT 的内存跑起来。
4. `rocm/comfyui` 在 gfx1151 上能不能用（它的文档只列数据中心卡），不加 `SYS_PTRACE`、`seccomp=unconfined`、`ipc=host` 能不能跑。
   不能用就是“上游没有合适的镜像”，按主人的规矩不自建，ComfyUI 先不做。
5. arm64 主机选到只有 amd64 的厂商版时，预览写明这家不能用，退到下一家或通用镜像。
6. 没有 NVIDIA 主机：NVIDIA 这条只有假 Docker 和假 sysfs 的行为测试，和 ADR-0018 一样。

## 实施顺序

1. 规范（wonton-docs 的“显卡”“对宿主的要求”“安装时注入的变量”“各版本新增的字段”）和清单解析、`plugin check`，一个 PR。
2. 核心：选厂商、换镜像、写进记录、`WONTON_GPU_VENDOR`、预览那一行和读平台列表；行为测试用假 sysfs 和假 Docker。
3. `check-index` 和官方索引的审核清单。
4. 第一个样例：Ollama 插件加 `amd` 的镜像，在一台 Strix Halo 服务器上跑上面的待验证项，结果写回这篇的“验证结果”。

## 关联

[ADR-0015](0015-small-components-standard-interfaces.md)、[ADR-0017](0017-plugin-ecosystem-and-store.md)、
[ADR-0018](0018-gpu-for-plugins.md)、[ADR-0033](0033-plugin-tabs.md)、[ADR-0036](0036-plugin-privileges.md)、
[ADR-0040](0040-registry-mirror.md)、[ADR-0057](0057-optional-permissions.md)

## 查证来源（2026-11-12）

- Ollama：仓库的 `docs/docker.mdx`、`docs/gpu.mdx`、`Dockerfile`（`ROCMVERSION=7.2.1`）；Docker Hub `ollama/ollama` 的 `0.40.16`、
  `0.40.16-rocm` 标签（摘要、平台）。
- llama.cpp：仓库的 `docs/docker.md`。
- vLLM：Docker Hub 的 `vllm/vllm-openai`、`vllm/vllm-openai-rocm`、`rocm/vllm` 标签。
- ComfyUI：Docker Hub 的 `rocm/comfyui`；AMD 文档“ComfyUI on ROCm installation”。
- ROCm：AMD 文档“Linux support matrices by ROCm version”（Radeon 和 Ryzen，7.2.1）、ROCm Core SDK 10.1.0 发行说明；
  AMD Container Toolkit 文档（instinct.docs.amd.com）。
