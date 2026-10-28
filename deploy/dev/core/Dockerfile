# 开发用的核心镜像：在容器里编译，宿主不需要交叉编译工具链。构建上下文是仓库根目录，
# 只放进 Cargo 工作区（见 Dockerfile.dockerignore）。
FROM rust:1.99.1-slim-trixie AS build
# 固定为镜像自带的工具链，不按 rust-toolchain.toml 再去下载组件。
ENV RUSTUP_TOOLCHAIN=1.99.1 CARGO_TERM_COLOR=never
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p wonton-core \
    && cp target/release/wonton-core /usr/local/bin/wonton-core

# 冒烟测试用的隧道客户端（wonton-tunnel 的示例），smoke.sh 用它扮演已配对的手机。只在
# `--target tunnel-client` 时构建，不影响核心镜像。
FROM build AS tunnel-client-build
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p wonton-tunnel --example tunnel-client \
    && cp target/release/examples/tunnel-client /usr/local/bin/tunnel-client

FROM debian:trixie-slim AS tunnel-client
COPY --from=tunnel-client-build /usr/local/bin/tunnel-client /usr/local/bin/tunnel-client
ENTRYPOINT ["tunnel-client"]

FROM debian:trixie-slim
COPY --from=build /usr/local/bin/wonton-core /usr/local/bin/wonton-core
ENTRYPOINT ["wonton-core"]
CMD ["serve"]
