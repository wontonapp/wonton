# C3 登录验收用的测试 Agent 镜像：入口脚本 + 无外部账号的假 ACP Agent。
# 入口脚本与 acp-bridge 样例共用一份（构建时经 entry 上下文传入）。
# 仅供 deploy/dev 验收，不是产品镜像；版本随仓库内容变化，不发布。
FROM public.ecr.aws/docker/library/debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data/agent/home /workspace /run/wonton \
    && groupadd --system --gid 65532 wonton \
    && useradd --system --uid 65532 --gid 65532 --no-create-home wonton \
    && chown -R 65532:65532 /data/agent /workspace /run/wonton

COPY --from=entry agent-entrypoint.py /usr/local/bin/wonton-acp-agent
COPY c3-fake-agent.py /usr/local/bin/c3-fake-agent
RUN chmod 0755 /usr/local/bin/wonton-acp-agent /usr/local/bin/c3-fake-agent

ENV HOME=/data/agent/home \
    WONTON_ACP_SOCKET=/run/wonton/acp.sock \
    WONTON_AGENT_WORKSPACE=/workspace \
    WONTON_AGENT_COMMAND=/usr/local/bin/c3-fake-agent

USER 65532:65532
ENTRYPOINT ["/usr/local/bin/wonton-acp-agent"]
