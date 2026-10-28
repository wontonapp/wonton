# 开发用的核心镜像：在容器里编译，宿主不需要交叉编译工具链。构建上下文是仓库根目录，
# 只放进 Cargo 工作区（见 Dockerfile.dockerignore）。
FROM rust:1.99.1-slim-trixie AS build
# 固定为镜像自带的工具链，不按 rust-toolchain.toml 再去下载组件。
ENV RUSTUP_TOOLCHAIN=1.99.1 CARGO_TERM_COLOR=never
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
    && cargo build --release --locked -p wonton-core \
    && cp target/release/wonton-core /usr/local/bin/wonton-core

# 冒烟测试用的隧道客户端（wonton-tunnel 的示例），smoke.sh 用它扮演已配对的手机。只在
# `--target tunnel-client` 时构建，不影响核心镜像。
FROM build AS tunnel-client-build
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    find crates -exec touch {} + \
    && cargo build --release --locked -p wonton-tunnel --example tunnel-client \
    && cp target/release/examples/tunnel-client /usr/local/bin/tunnel-client

FROM debian:trixie-slim AS tunnel-client
COPY --from=tunnel-client-build /usr/local/bin/tunnel-client /usr/local/bin/tunnel-client
ENTRYPOINT ["tunnel-client"]

FROM debian:trixie-slim
COPY --from=build /usr/local/bin/wonton-core /usr/local/bin/wonton-core
ENTRYPOINT ["wonton-core"]
CMD ["serve"]
