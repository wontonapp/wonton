# ACP bridge 发布镜像：桥只编译 Rust 二进制；Agent 镜像由每个 Agent 插件负责。
# 基础镜像默认从 ECR Public 拉（国内的构建机连不上 Docker Hub）；镜像冒烟测试经 build-arg 指到
# ghcr.io/<owner>/test-images 的副本（deploy/dev/acp-bridge 的 Dockerfile 是测试唯一构建的）。
ARG RUST_IMAGE=public.ecr.aws/docker/library/rust:1.99.1-slim-trixie
ARG DEBIAN_IMAGE=public.ecr.aws/docker/library/debian:trixie-slim
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS build-base
ENV RUSTUP_TOOLCHAIN=1.99.1 CARGO_TERM_COLOR=never
ARG TARGETARCH BUILDARCH CARGO_BUILD_JOBS
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
RUN case "$TARGETARCH" in \
      amd64) triple=x86_64-unknown-linux-gnu gcc=gcc-x86-64-linux-gnu ;; \
      arm64) triple=aarch64-unknown-linux-gnu gcc=gcc-aarch64-linux-gnu ;; \
      *) echo "不支持的架构 $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && echo "$triple" >/rust-target \
    && if [ "$TARGETARCH" != "$BUILDARCH" ]; then \
         apt-get update && apt-get install -y --no-install-recommends "$gcc" "libc6-dev-$TARGETARCH-cross" \
         && rm -rf /var/lib/apt/lists/* && rustup target add "$triple"; \
       fi
# cargo-chef（Apache-2.0 OR MIT），依赖层用。
RUN cargo install cargo-chef --locked --version 0.1.78

FROM build-base AS planner
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY vendor vendor
# recipe 里记着每个工作区 crate 的全部 target，自动发现的测试、示例、基准也在内：哪个 crate 加一个测试文件
# recipe 就变，依赖层（4 核约 330 s）跟着重编（11-12 实测 main 上 80 次合并里 7 次，去掉后 3 次）。
# 桥的 release 构建不编这三类 target，依赖解析也不看它们，删了再生成 recipe 不影响 cook 的内容。
RUN rm -rf crates/*/tests crates/*/examples crates/*/benches \
    && cargo chef prepare --recipe-path recipe.json

# 依赖层：只拿 recipe.json 和 vendor（[patch] 的源码，cook 解析要用），清单不变就命中缓存。cook 按桩工程
# 改它自己的 Cargo.lock（带 --locked 时 cargo 拒绝，实测），下面的正式构建把上下文里原始的 Cargo.lock
# COPY 回来，--locked 照常把门。
FROM build-base AS deps
WORKDIR /src
COPY --from=planner /src/recipe.json recipe.json
COPY vendor vendor
# cook 必须和下面的 build 用同一个 -p：cook 整个工作区时依赖按全工作区统一 feature，build -p 的 feature 集合不同，最后一层要重编 128 个外部 crate（4 核实测 297 s；加 -p 后只编 5 个工作区 crate，92–105 s）。
# 发布镜像不带符号表，镜像小约 30 MB。strip 是 profile 的一部分，cook 和下面的 build 必须一致，
# 不一致时 cargo 判定依赖全都过期：依赖层命中了，最后一层照样重编 337 个依赖（4 核 559 s，实测）。
ENV CARGO_PROFILE_RELEASE_STRIP=symbols
RUN cargo chef cook --release -p wonton-acp-bridge --recipe-path recipe.json --target "$(cat /rust-target)"

FROM deps AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY vendor vendor
RUN cargo build --release --locked --target "$(cat /rust-target)" -p wonton-acp-bridge \
    && cp "target/$(cat /rust-target)/release/wonton-acp-bridge" /usr/local/bin/wonton-acp-bridge

FROM ${DEBIAN_IMAGE} AS bridge
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/wonton-acp-bridge /usr/local/bin/wonton-acp-bridge
COPY deploy/dev/acp-bridge/entrypoint.sh /usr/local/bin/wonton-acp-bridge-entrypoint
ENTRYPOINT ["wonton-acp-bridge-entrypoint"]
