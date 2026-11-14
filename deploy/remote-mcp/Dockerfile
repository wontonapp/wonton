# 原生 amd64 / arm64 runner 构建 musl 静态二进制；不经 QEMU 或交叉 C 工具链。
ARG RUST_IMAGE=public.ecr.aws/docker/library/rust:1.99.1-slim-trixie
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS build-base
ENV RUSTUP_TOOLCHAIN=1.99.1 CARGO_TERM_COLOR=never
ARG TARGETARCH BUILDARCH CARGO_BUILD_JOBS
RUN test "$TARGETARCH" = "$BUILDARCH" \
    && case "$TARGETARCH" in \
         amd64) triple=x86_64-unknown-linux-musl ;; \
         arm64) triple=aarch64-unknown-linux-musl ;; \
         *) echo "不支持的架构 $TARGETARCH" >&2; exit 1 ;; \
       esac \
    && echo "$triple" >/rust-target \
    && apt-get update \
    && apt-get install -y --no-install-recommends musl-tools cmake make ca-certificates binutils \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add "$triple"
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
    CC_x86_64_unknown_linux_musl=musl-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
    CC_aarch64_unknown_linux_musl=musl-gcc \
    CARGO_PROFILE_RELEASE_STRIP=symbols
# cargo-chef（Apache-2.0 OR MIT），与桥镜像复用依赖层写法。
RUN cargo install cargo-chef --locked --version 0.1.78

FROM build-base AS planner
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY vendor vendor
RUN rm -rf crates/*/tests crates/*/examples crates/*/benches \
    && cargo chef prepare --recipe-path recipe.json

FROM build-base AS deps
WORKDIR /src
COPY --from=planner /src/recipe.json recipe.json
COPY vendor vendor
RUN cargo chef cook --release -p wonton-remote-mcp --recipe-path recipe.json --target "$(cat /rust-target)"

FROM deps AS build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY vendor vendor
RUN cargo build --release --locked --target "$(cat /rust-target)" -p wonton-remote-mcp \
    && cp "target/$(cat /rust-target)/release/wonton-remote-mcp" /wonton-remote-mcp \
    && if readelf -l /wonton-remote-mcp | grep -q INTERP; then \
         echo "连接器不能依赖动态加载器" >&2; exit 1; \
       fi \
    && if readelf -d /wonton-remote-mcp | grep -q NEEDED; then \
         echo "连接器不能依赖动态库" >&2; exit 1; \
       fi \
    && mkdir /data

FROM scratch AS remote-mcp
COPY --from=build /wonton-remote-mcp /usr/local/bin/wonton-remote-mcp
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /data /data
ENV WONTON_REMOTE_MCP_LISTEN=0.0.0.0:8080
WORKDIR /data
EXPOSE 8080
# 由插件包挂载唯一的数据卷；不声明 VOLUME，避免产生核心未管理的匿名卷。
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s \
    CMD ["/usr/local/bin/wonton-remote-mcp", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/wonton-remote-mcp"]
