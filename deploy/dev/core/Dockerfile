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

FROM debian:trixie-slim
COPY --from=build /usr/local/bin/wonton-core /usr/local/bin/wonton-core
ENTRYPOINT ["wonton-core"]
CMD ["serve"]
