# 文件搜索插件的镜像：rust-mcp-filesystem（MIT）在挂进来的共享文件夹里找文件、读文件，只说 stdio；supergateway（MIT）
# 把它转成 Streamable HTTP，核心的 MCP 总线经插件网络连它。两个都用上游的发行版，不改代码；版本和校验和一起升级。
FROM node:24-bookworm-slim@sha256:c86ecead9e55a5210ac1727de38a4ca840bf03dcaf34dc9749addbf383b1920c

ARG TARGETARCH
ARG FILESYSTEM_VERSION=0.4.5
ARG FILESYSTEM_SHA256_AMD64=4c6a7fd06f1e55bd2928610b0b6ecb6e8589ca816fb7ea17b6a2b2f36a30e00c
ARG FILESYSTEM_SHA256_ARM64=79f0022e86532b74a6ef02e3b5745d90bc50852dd6d1c47414e0a6a292f6a310
ARG SUPERGATEWAY_VERSION=4.2.1

RUN set -eux; \
    case "$TARGETARCH" in \
      amd64) arch=x86_64; sum="$FILESYSTEM_SHA256_AMD64" ;; \
      arm64) arch=aarch64; sum="$FILESYSTEM_SHA256_ARM64" ;; \
      *) echo "不支持的架构 $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl; \
    name="rust-mcp-filesystem-$arch-unknown-linux-gnu"; \
    curl -fsSLo "/tmp/$name.tar.gz" \
      "https://github.com/rust-mcp-stack/rust-mcp-filesystem/releases/download/v$FILESYSTEM_VERSION/$name.tar.gz"; \
    echo "$sum  /tmp/$name.tar.gz" | sha256sum -c -; \
    tar -xzf "/tmp/$name.tar.gz" -C /tmp; \
    install -m 0755 "/tmp/$name/rust-mcp-filesystem" /usr/local/bin/rust-mcp-filesystem; \
    rm -rf /tmp/*; \
    apt-get purge -y curl; \
    apt-get autoremove -y; \
    rm -rf /var/lib/apt/lists/*; \
    npm install -g "supergateway@$SUPERGATEWAY_VERSION"; \
    npm cache clean --force

USER node
ENTRYPOINT ["supergateway"]
