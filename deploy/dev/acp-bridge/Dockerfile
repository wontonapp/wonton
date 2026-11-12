# ACP bridge 发布镜像：桥只编译 Rust 二进制；Agent 镜像由每个 Agent 插件负责。
FROM --platform=$BUILDPLATFORM public.ecr.aws/docker/library/rust:1.99.1-slim-trixie AS build
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
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    find crates -exec touch {} + \
    && cargo build --release --locked --target "$(cat /rust-target)" -p wonton-acp-bridge \
    && cp "target/$(cat /rust-target)/release/wonton-acp-bridge" /usr/local/bin/wonton-acp-bridge

FROM public.ecr.aws/docker/library/debian:trixie-slim AS bridge
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/wonton-acp-bridge /usr/local/bin/wonton-acp-bridge
COPY deploy/dev/acp-bridge/entrypoint.sh /usr/local/bin/wonton-acp-bridge-entrypoint
ENTRYPOINT ["wonton-acp-bridge-entrypoint"]
