FROM public.ecr.aws/docker/library/debian:trixie-slim

ARG TARGETARCH
ARG OMP_VERSION=18.16.0
ARG OMP_X64_SHA256=6498ed785bab00d2926e0387e0ec02362d6d45f1c3d0d1b57c1618b7f95c9264
ARG OMP_ARM64_SHA256=c2be1ea85ce17c4270854b57efff2a7ea1d5a03c7557fd08721f2b9bb32698de

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl python3 passwd \
    && rm -rf /var/lib/apt/lists/*
RUN case "$TARGETARCH" in \
      amd64) asset=omp-linux-x64; checksum="$OMP_X64_SHA256" ;; \
      arm64) asset=omp-linux-arm64; checksum="$OMP_ARM64_SHA256" ;; \
      *) echo "不支持的架构 $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && curl --fail --silent --show-error --location \
         "https://github.com/can1357/oh-my-pi/releases/download/v${OMP_VERSION}/${asset}" \
         --output /usr/local/bin/omp \
    && echo "$checksum  /usr/local/bin/omp" | sha256sum --check --strict \
    && chmod 0755 /usr/local/bin/omp \
    && mkdir -p /data/agent/home /workspace /run/wonton /usr/share/doc/wonton-acp-agent \
    && groupadd --system --gid 65532 wonton \
    && useradd --system --uid 65532 --gid 65532 --no-create-home wonton \
    && chown -R 65532:65532 /data/agent /workspace /run/wonton

COPY deploy/dev/acp-bridge/agent-entrypoint.py /usr/local/bin/wonton-acp-agent
COPY deploy/dev/acp-bridge/LICENSE.omp /usr/share/doc/wonton-acp-agent/LICENSE
RUN chmod 0755 /usr/local/bin/wonton-acp-agent \
    && chmod 0644 /usr/share/doc/wonton-acp-agent/LICENSE

ENV HOME=/data/agent/home \
    WONTON_ACP_SOCKET=/run/wonton/acp.sock \
    WONTON_AGENT_WORKSPACE=/workspace

USER 65532:65532
ENTRYPOINT ["/usr/local/bin/wonton-acp-agent"]
