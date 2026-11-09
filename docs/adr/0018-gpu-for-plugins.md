# ADR-0018 显卡交给插件：核心按宿主的显卡给设备

- 状态：已接受（2026-10-30）

## 背景

主人想在有显卡的机器上装好 Wonton 后，用插件生图、生视频、跑 ComfyUI 工作流和各种能在本地跑的开源模型。
这些插件各在自己的仓库里（ADR-0017），核心先得能把显卡交给插件的容器（#80 的前提）。此前清单能解析 `devices.gpu` 权限和
`requirements.gpu`，但 compose 的 `devices` 被整个拒绝，建容器时也不给任何设备。

Docker 把显卡交给容器有两条路：`--device` 把内核的设备节点放进容器，AMD、Intel 的开源驱动走这条，ROCm 的文档要求
`--device=/dev/kfd --device=/dev/dri` 再加 `video`、`render` 组；`--gpus`（Engine API 的 `DeviceRequests`）由设备驱动插件兑现，
常见的实现只有 NVIDIA Container Toolkit，它在容器启动时把 NVIDIA 的设备节点和驱动库一起放进去。

## 决策

- **插件只说要不要、给谁，核心决定给什么。** 清单申请 `devices.gpu`（高权限，主人醒目确认），`gpu.services` 列出拿到显卡的
  服务，`requirements.gpu` 是 `required`（默认）或 `optional`。compose 里自己写的 `devices` 仍然拒绝：插件写不了宿主路径，
  一份清单在 AMD 和 NVIDIA 的主机上都能装。写法见插件规范的“显卡”。
- **按宿主实际有的显卡给。**
  - AMD、Intel 等开源驱动（sysfs 里有 DRM 渲染节点）：`/dev/dri` 整个目录，有 KFD（amdgpu 的 ROCm 计算接口）时还有
    `/dev/kfd`，容器里路径不变。容器加入这些设备节点所属的组：GID 直接从宿主的节点读（通常就是 `video`、`render`），不按
    组名在容器里解析，容器镜像里 `video` 是另一个 GID。
  - NVIDIA 官方驱动（内核模块 `nvidia` 已加载）：`DeviceRequests`，`Count` 为 -1、capability `gpu`，同
    `docker run --gpus all`，由 NVIDIA Container Toolkit 放进全部显卡。核心另外确认 Docker 登记了 `nvidia` 运行时，
    `nvidia-ctk runtime configure --runtime=docker` 就是装它。
  - 两种都有时都给。不给 `privileged`、`cap_add`、`security_opt`、`ipc: host`，共享内存由插件写 `shm_size`。
- **失败即停。** `required` 的插件在没有可用显卡的主机上装不上，错误类别 `gpu_unavailable`，原因分四种：没有找到显卡
  （PCI 上没有显示设备）；有显卡但没有可用的驱动（没绑定驱动，或者是 nouveau）；有 NVIDIA 驱动但 Docker 没有 `nvidia`
  运行时；核心在容器里却没挂载宿主的 `/dev`。`optional` 的照常装、不给设备，预览里写明。判断看 sysfs 的驱动和设备节点，
  不看 `nvidia-smi`、`rocminfo` 装没装：开发机装着 `nvidia-smi`，却没有 NVIDIA 驱动。
- **核心只读、不递归地看宿主的 `/dev`。** sysfs 是内核的，核心在容器里看到的和宿主一样，够判断有没有显卡、加载了什么
  驱动；设备节点属于哪个组只有 `/dev` 知道。正式安装和开发环境的 compose 都把宿主的 `/dev` 挂到核心容器的 `/host/dev`
  （`WONTON_HOST_DEV`），`read_only` 加 `bind.recursive: disabled`：`/dev` 下的子挂载不带进来（`/dev/shm` 里是宿主其他进程的
  共享内存，桌面主机上有 Wayland、PipeWire 的缓冲；还有 `/dev/pts`、`/dev/mqueue`），核心只要 devtmpfs 里的设备节点，
  `/dev/dri`、`/dev/kfd` 不是单独的挂载。设备 cgroup 没有放行，核心开不了这些设备；核心本来就持有 Docker socket
  （ADR-0011），这不增加它的权限，但读文件的路径要是没收住，也读不到宿主的共享内存。核心直接跑在宿主上时读 `/dev`。
- **要什么样的显卡，插件自己写，核心判断。** `requirements.gpu` 可以限定厂商（`vendors`）和显存下限（`memory`），写法见插件规范
  的“对宿主的要求”。商店、预览和安装都按这台主机实际的显卡判断：满足、不满足（`required` 的装不了，`optional` 的不给设备）、
  查不出来（如实显示，不拦安装）。限定了厂商时只给那家的：NVIDIA 和别家混装的主机上不多给另一家。AMD 和 Intel 混装时分不开，
  都在 `/dev/dri` 里。
- **显存各家读法不同，读不到的不猜。** AMD 独显、Intel 独显读 sysfs。NVIDIA 的不在 sysfs 里，用核心的镜像起一次性容器，用插件拿
  显卡时一样的 `DeviceRequests`，问 `nvidia-smi`（Toolkit 带进去的，`NVIDIA_DRIVER_CAPABILITIES=utility`），顺便证明容器里
  真能用上；问到了这个核心进程里不再问，问不到隔一分钟再问。AMD 集成显卡报的是划给它的那部分内存，不是上限，比要求小时算查不
  出来。标称和实际差一点，到要求的 90% 就算够，和主机内存一样。
- **预览和确认写明给了什么。** 安装预览的说明（`InstallPreview.notices`）里有一行：拿显卡的服务，本机会给什么（厂商、设备、
  组）或者为什么给不了；`required` 而本机给不了时，原因同时写进 `blocked`，和其他“这台主机上装不了”的原因一样。确认卡片
  列出拿显卡的服务。

## 没选的做法

- 放开 compose 的 `devices`，由插件写设备路径：主人确认的只是“用显卡”，看不出给了什么；AMD 和 NVIDIA 的写法不同，一份清单
  装不了两种主机；插件也能借 `devices` 要走别的设备。
- 一次性探测容器挂着宿主的 `/dev` 去读设备：不用改核心的 compose，但多一套镜像和容器的生命周期；挂载一行就解决。
- 只挂 `/dev/dri`、`/dev/kfd`：没有显卡的主机上 compose 会把缺失的源路径建成目录，或者核心起不来。
- 递归地挂 `/dev`（短写法 `/dev:/host/dev:ro`）：宿主 `/dev` 下的子挂载都会带进核心容器，Docker 25 之前或内核低于 5.12 时
  这些子挂载还是可写的。
- `--group-add video`、`render` 按名字：Docker 在容器的 `/etc/group` 里解析，Ubuntu 镜像里 `video` 是 44，和宿主对不上。
- 等 CDI（`nvidia.com/gpu=all`）：Docker 28 起支持，但要主人先生成 CDI 规范；`--gpus all` 的 `DeviceRequests` 是 NVIDIA
  Container Toolkit 文档的标准做法，装好工具包就能用。CDI 的写法等有 NVIDIA 主机实测后再定。

## 代价与风险

- NVIDIA 这条路实测到 `--gpus all` 的容器里跑 `nvidia-smi`（Docker 28.2.2、一块 NVIDIA 显卡）；Intel 显卡会拿到 `/dev/dri`，没有实测。
- `/dev/dri` 整个目录连显示节点（`card*`）也给了，插件能改显示设置；ROCm 的文档也是整个目录。
- 装好后核心不再盯着显卡：显卡拔了、驱动换了，容器自己报错。
- 显卡不能按块分配：多块显卡全部交给每个声明的服务。
- 核心容器多看到宿主 `/dev` 里的设备节点（只读，不含子挂载）。

## 验证结果（2026-10-30）

开发机：Arch Linux，内核 7.2.3，AMD 的 APU 带集成显卡（amdgpu，有 KFD），
Docker Engine 29.7.2，只有 `runc` 运行时；装着 `nvidia-smi`，没有 NVIDIA 驱动。宿主上 `/dev/kfd` 属于 `render`（987，0666），
`/dev/dri/card1` 属于 `video`（983，0660），`/dev/dri/renderD128` 属于 `render`（0666）。

- 核心直接跑在宿主上（没有 `WONTON_CORE_CONTAINER`，读 `/dev`），经管理接口装 `plugins/examples/gpu-check`（容器以
  `1000:1000` 运行）：预览的 `gpu` 一节是“AMD 显卡：/dev/kfd、/dev/dri，容器加入组 983、987”；安装 1.8 秒，健康。容器的
  `HostConfig`：`Devices` 是 `/dev/kfd`、`/dev/dri`（`rwm`），`GroupAdd` 是 983、987，没有 `DeviceRequests`、`Privileged`、
  `CapAdd`、`SecurityOpt`。容器里 `id` 带着组 983、987，ROCm 7.2.4 的 `rocminfo` 报出 GPU agent `gfx1151`（40 个计算单元）。
- 核心在容器里（`deploy/dev/core` 的开发环境，Compose v5.5.0，核心镜像按本分支构建）：compose 认
  `bind.recursive: disabled`，核心容器的挂载是 `ReadOnly` 加 `BindOptions.NonRecursive`，`/host/dev` 是只读的 devtmpfs，
  `/host/dev/shm`、`pts`、`mqueue`、`hugepages` 都是空目录，`/host/dev/kfd` 照常能读到属主。`dev.sh install
  plugins/examples/gpu-check/docker-compose.yml` 的预览打出“提示：显卡交给服务 check：AMD 显卡：/dev/kfd、/dev/dri，容器加入组
  983、987”，装好健康，容器的 `HostConfig` 和上一条相同。
- 同一个插件在 compose 里写 `devices` 时预览被拒（400 `invalid_input`），错误指明改用 `gpu.services`。
- 失败即停：第一版检查镜像用 Ubuntu 24.04 自带的 rocminfo（ROCm 5.7），认不出 gfx1151（“The GPU node has an unrecognized
  id”），健康检查没过，安装 51 秒后失败并回滚，错误里附着 rocminfo 的输出；换成 AMD 仓库里的 ROCm 7.2.4 后通过。设备交
  对了还要容器里的 ROCm 认得这块显卡，插件镜像要用够新的 ROCm。
- 卸载（删数据）后没有残留的容器和网络。
- 没有 NVIDIA、Intel 主机：这两条路径只有假 Docker 和假 sysfs 的行为测试。
