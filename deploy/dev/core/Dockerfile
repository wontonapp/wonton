# 核心镜像，开发（dev.sh、冒烟）和发布（.github/workflows/release.yml）共用：在容器里编译，宿主不需要工具链。
# 构建上下文是仓库根目录，只放进 Cargo 工作区（见 Dockerfile.dockerignore）。
#
# 编译总在构建机自己的架构上跑，要别的架构时交叉编译（`--platform linux/arm64`），不在 QEMU 里跑编译器；核心镜像的
# 最后一层只复制文件、不执行命令，也用不到模拟器。
#
# 基础镜像是 Docker 官方镜像在 ECR Public 上的副本：国内的构建机（比如跑发布作业的 npchy）连不上 Docker Hub，
# ECR Public 直连可用（2026-10-29 实测）。
FROM --platform=$BUILDPLATFORM public.ecr.aws/docker/library/rust:1.99.1-slim-trixie AS build
# 固定为镜像自带的工具链，不按 rust-toolchain.toml 再去下载组件。
ENV RUSTUP_TOOLCHAIN=1.99.1 CARGO_TERM_COLOR=never
ARG TARGETARCH BUILDARCH
# 和别的作业共用一台机器时用 `--build-arg CARGO_BUILD_JOBS=…` 限制并行编译的任务数，不给就按 CPU 数。
ARG CARGO_BUILD_JOBS
# Debian 给本机的 gcc 也装了带三元组前缀的名字，同一套设置在本机编译和交叉编译时都能用。
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
RUN case "$TARGETARCH" in \
      amd64) triple=x86_64-unknown-linux-gnu gcc=gcc-x86-64-linux-gnu ;; \
      arm64) triple=aarch64-unknown-linux-gnu gcc=gcc-aarch64-linux-gnu ;; \
      *) echo "不支持的架构 $TARGETARCH，只支持 amd64 和 arm64" >&2; exit 1 ;; \
    esac \
    && echo "$triple" >/rust-target \
    && if [ "$TARGETARCH" != "$BUILDARCH" ]; then \
         apt-get update && apt-get install -y --no-install-recommends "$gcc" "libc6-dev-$TARGETARCH-cross" \
         && rm -rf /var/lib/apt/lists/* && rustup target add "$triple"; \
       fi
WORKDIR /src
COPY . .
# 构建它的提交号（dev.sh 传入），编进核心、打在启动日志里，一眼看出镜像里是哪份代码。
ARG WONTON_BUILD_COMMIT=unknown
# target 缓存被所有 worktree 共用，而 cargo 按修改时间判断工作区的 crate 要不要重编：这份源码比别的 worktree
# 上次编译时还旧，cargo 就直接用别人的产物，镜像里装的是别的分支的代码。所以每次编译前把工作区的源码 touch
# 一遍，在同一个持锁的 RUN 里：依赖按版本判断，仍然命中缓存，只重编工作区的 crate。
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    find crates -exec touch {} + \
    && cargo build --release --locked --target "$(cat /rust-target)" -p wonton-core \
    && cp "target/$(cat /rust-target)/release/wonton-core" /usr/local/bin/wonton-core

# 冒烟测试用的隧道客户端（wonton-tunnel 的示例），smoke.sh 用它扮演已配对的手机。只在
# `--target tunnel-client` 时构建，不影响核心镜像。
FROM build AS tunnel-client-build
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    find crates -exec touch {} + \
    && cargo build --release --locked --target "$(cat /rust-target)" -p wonton-tunnel --example tunnel-client \
    && cp "target/$(cat /rust-target)/release/examples/tunnel-client" /usr/local/bin/tunnel-client

FROM public.ecr.aws/docker/library/debian:trixie-slim AS tunnel-client
COPY --from=tunnel-client-build /usr/local/bin/tunnel-client /usr/local/bin/tunnel-client
ENTRYPOINT ["tunnel-client"]

# 回声 Bot 示例插件（plugins/examples/echo-bot）的镜像，冒烟测试也用它扮演主人的加密客户端。只在
# `--target echo-bot` 时构建。
FROM build AS echo-bot-build
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    find crates -exec touch {} + \
    && cargo build --release --locked --target "$(cat /rust-target)" -p wonton-echo-bot \
    && cp "target/$(cat /rust-target)/release/echo-bot" /usr/local/bin/echo-bot

FROM public.ecr.aws/docker/library/debian:trixie-slim AS echo-bot
# matrix-sdk 的 HTTP 客户端启动时要加载系统的根证书，没有就直接失败（即使只用 http）。
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=echo-bot-build /usr/local/bin/echo-bot /usr/local/bin/echo-bot
ENTRYPOINT ["echo-bot"]
CMD ["run"]

FROM public.ecr.aws/docker/library/debian:trixie-slim AS core
# 推送代理把推送发到设备给的地址，可能是 https，要系统的根证书；HTTP 客户端在核心启动时就加载它们。根证书从编译阶段
# 复制过来，这一层不执行命令：交叉编译的 arm64 镜像不用模拟器也能构建。
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /usr/local/bin/wonton-core /usr/local/bin/wonton-core
# 随核心预装的系统插件（WONTON_SYSTEM_PLUGINS）：模型网关 magpie。
COPY plugins/magpie /usr/share/wonton/system-plugins/magpie
# 随核心附带、只自动装一次的默认插件（WONTON_DEFAULT_PLUGINS）：默认 Agent Hermes。
COPY plugins/hermes /usr/share/wonton/default-plugins/hermes
ENTRYPOINT ["wonton-core"]
CMD ["serve"]

# 开发和冒烟用的核心镜像（`--target dev`）：多一个开发用的插件目录（WONTON_CATALOG），Bot 经 MCP 总线只能装这里的
# 插件（ADR-0016）。发布的镜像不带它。
FROM core AS dev
COPY plugins/files /usr/share/wonton/catalog/files
COPY plugins/examples/whoami /usr/share/wonton/catalog/whoami

# 发布的核心镜像：release.yml 不指定 target，构建的是最后这个阶段。
FROM core
