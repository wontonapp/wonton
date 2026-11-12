# 显卡检查

验证核心把宿主的显卡交给插件（插件规范的“显卡”、ADR-0018）：容器以 `PUID:PGID` 运行，启动时打印身份和设备节点，
有 `/dev/kfd` 就跑 `rocminfo`，有 `nvidia-smi` 就跑它；报出显卡才算健康，否则安装失败，错误里附上输出。

```bash
docker build -t wonton-gpu-check:dev plugins/examples/gpu-check
```

```bash
deploy/dev/core/dev.sh install plugins/examples/gpu-check
```

镜像里的 `rocminfo` 来自 AMD 的 apt 仓库（ROCm 的最新版本）：Ubuntu 自带的是 ROCm 5.7，认不出新显卡。构建机要经
代理下载时加 `--network host --build-arg http_proxy=… --build-arg https_proxy=…`。装好后
`docker logs wonton-app-wonton-gpu-check--check` 是容器里看到的显卡；`docker inspect` 里的 `HostConfig.Devices`、
`GroupAdd`（或 NVIDIA 的 `DeviceRequests`）是核心交给它的。

## 图标

`icon.svg` 的来源及许可见[自制图标](../../README.md#自制图标)。
