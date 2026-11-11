# ADR-0002 部署形态：任意 Linux + Docker

- 状态：已接受（2026-10-28）；“不支持 macOS 或 Windows 作为宿主”一条被 [ADR-0062](0062-desktop-vm-host.md) 部分取代

## 背景

旧方案往通用 Linux 发行版上 `curl | bash` 装软件，再用 bash 写的 updater 在可变系统上实现事务、
快照和回滚。发行控制仓里约 4.6 万行 shell/mjs，大部分用于签名 OTA、installer 和事务提交，
发布事故频发。

## 决策

- 服务端只支持装有 Docker 的 Linux（amd64、arm64），包括群晖、飞牛、Unraid、TrueNAS 等已有 NAS。
- 核心本身是一个容器，通过 Docker API 管理所有插件容器；一行命令安装。
- 不做自有 OS 镜像，不支持 macOS 或 Windows 作为宿主。

## 理由

- 运行时、隔离、版本和回滚交给成熟的容器工具，不再自己实现。
- 覆盖面最大：已有 NAS 和小主机的用户都能直接装。
- 镜像以 digest 标识版本，天然是不可变制品。

## 代价与风险

- 依赖宿主 Docker；需要宿主设备（GPU、USB）的插件要申请高权限。
- 没有整机 A/B 升级，宿主系统本身的维护由用户或 NAS 系统负责。
- 核心容器需要 Docker socket，等同于宿主 root 权限：核心必须保持小而可审查，插件默认不得访问 socket。

## 待验证（M0 / M1）

- 在群晖、飞牛等 NAS 的 Docker 环境里的安装体验和权限限制。
- GPU 直通（NVIDIA、AMD ROCm）给 Ollama 等插件的方式：见 ADR-0018。

## 验证结果（M1，一行安装）

`deploy/install/install.sh` 在干净的 Ubuntu 24.04 上装好核心、Hydra、Tuwunel，核心随后装上 magpie，四个容器都健康。
验证条件（2026-10-29）：OrbStack 虚拟机，arm64 原生、amd64 在 Rosetta 下，Docker Engine 29.9.2、Compose v5.6.1；核心镜像
用发布的 Dockerfile 在一台 Linux 主机上构建（arm64 交叉编译）后导入虚拟机，Hydra、Tuwunel、magpie 的镜像从各自的仓库拉取；安装
脚本由本机的 HTTP 服务提供，按 `curl … | sudo sh` 执行。

- amd64：从执行安装命令到终端出现首次配对码（这时核心、Hydra、Tuwunel 都健康）465 秒，大半是在 Rosetta 下拉 Hydra 和
  Tuwunel 的镜像；magpie 在核心启动 4 分半后装好。上一轮不含拉镜像时 arm64 16 秒、amd64 34 秒。
- 安装终端最后显示首次配对码，和状态页上的是同一个。arm64 上 iOS 模拟器里的 App 粘贴这个码配上，amd64 上隧道客户端
  示例用它配上；终端随即打出“配对成功”，安装命令退出，状态页只剩运行状态。App 配上后经隧道登录了主人的 Matrix 账号。
- 已配对的 App 里“添加设备”拿到的码、服务器上 `sudo wonton pair` 打出的码，各让一台设备（隧道客户端示例）配上。
- 再执行一次安装命令：arm64 2 秒、amd64 6 秒，密钥都沿用，已配对的设备都在，终端只说明怎么加设备。
- 主机上不需要 openssl：密钥由核心镜像里的 `wonton-core init` 生成。

### 公开发布后

用公开仓库的一行命令（`curl -fsSL https://raw.githubusercontent.com/geraldpeng6/wonton-install/main/install.sh | sudo sh`）
装发布出来的 `v0.1.0-beta.1`（beta 渠道）。验证条件（2026-10-30）：同上的 OrbStack 虚拟机，arm64 原生、amd64 在 Rosetta 下，
Docker Engine 29.9.2、Compose v5.6.1，所有镜像都从公开的仓库拉取。

- 从执行安装命令到终端出现首次配对码（含拉全部镜像）：arm64 44 秒，amd64 35 秒。
- `.env` 记下渠道（`WONTON_CORE_SOURCE=ghcr.io/wontonapp/wonton-core:beta`）和按 digest 写下的镜像
  （`ghcr.io/wontonapp/wonton-core@sha256:93feeb5de86afb2c678cef8e3c630cd4f6c0228d95aa9d046c58120ba6fe8b7f`）；amd64 上
  拉到的是 amd64 的变体。
- 核心、Hydra、Tuwunel、magpie 四个容器都健康。
- 从另一台机器用虚拟机的 IP 打开 8480 端口，看到运行状态和首次配对二维码；`Host` 换成域名得到 404。
- `sudo wonton pair` 打出一个配对码。
