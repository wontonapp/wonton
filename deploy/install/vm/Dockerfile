# 演练虚拟机的 qemu 容器：只给 /dev/kvm，不要特权、不挂 docker.sock。镜像校验（gpgv + Ubuntu 的签名公钥）、
# cloud-init 种子盘（genisoimage）、ssh 密钥都在容器里做，宿主上只要 Docker。
ARG BASE=debian:trixie-slim
FROM ${BASE}
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        qemu-system-x86 qemu-utils genisoimage curl ca-certificates gpgv ubuntu-keyring openssh-client \
    && rm -rf /var/lib/apt/lists/*
