# 显卡检查镜像：AMD 自己的 apt 仓库里的 rocminfo（ROCm 运行时，够报出 KFD 看到的显卡）。Ubuntu 自带的 rocminfo 是
# ROCm 5.7，认不出 2025 年起的显卡（Strix Halo 的 gfx1151 报“unrecognized id”），所以不用它。NVIDIA 主机上的
# nvidia-smi 由 NVIDIA Container Toolkit 在容器启动时放进来，镜像里不用装。基础镜像走 ECR Public：国内的构建机连不上
# Docker Hub，同核心的 Dockerfile。
FROM public.ecr.aws/docker/library/ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl gnupg \
    && mkdir -p /etc/apt/keyrings \
    && curl -fsSL https://repo.radeon.com/rocm/rocm.gpg.key | gpg --dearmor -o /etc/apt/keyrings/rocm.gpg \
    && echo "deb [arch=amd64 signed-by=/etc/apt/keyrings/rocm.gpg] https://repo.radeon.com/rocm/apt/latest noble main" \
       > /etc/apt/sources.list.d/rocm.list \
    && printf 'Package: *\nPin: release o=repo.radeon.com\nPin-Priority: 600\n' > /etc/apt/preferences.d/rocm \
    && apt-get update && apt-get install -y --no-install-recommends rocminfo \
    && rm -rf /var/lib/apt/lists/*
ENV PATH=/opt/rocm/bin:$PATH
COPY check.sh /usr/local/bin/gpu-check
ENTRYPOINT ["gpu-check"]
