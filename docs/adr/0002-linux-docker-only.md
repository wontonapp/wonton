# ADR-0002 部署形态：任意 Linux + Docker

- 状态：已接受（2026-10-28）

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
- GPU 直通（NVIDIA、AMD ROCm）给 Ollama 等插件的方式。
