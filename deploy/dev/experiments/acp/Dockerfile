# syntax=docker/dockerfile:1
# ACP 验证实验（#43）：每个 target 是一个 Agent 插件容器的最小形态，Node 24 加一个说 ACP 的程序。
# 版本全部固定，复跑结果才可比。探针脚本不进镜像，运行时只读挂载，镜像体积只算 agent 本身。
FROM node:24-bookworm-slim AS base
ENV NPM_CONFIG_UPDATE_NOTIFIER=false NPM_CONFIG_FUND=false NPM_CONFIG_AUDIT=false
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /workspace /state /out \
 && chown node:node /workspace /state /out
USER node
WORKDIR /workspace

# Claude Code：Zed 起头、现归 ACP 组织维护的适配器，内含 Claude Agent SDK 和 claude 原生程序（专有许可）。
FROM base AS claude
USER root
RUN npm install -g @agentclientprotocol/claude-agent-acp@0.98.0 && npm cache clean --force
USER node

# Codex：ACP 组织维护的适配器，内含 @openai/codex（Apache-2.0），经 codex app-server 驱动。
FROM base AS codex
USER root
RUN npm install -g @agentclientprotocol/codex-acp@2.3.0 && npm cache clean --force
USER node

# Kimi Code CLI（MIT），原生 ACP：kimi acp。Python 版 kimi-cli 已归档。
FROM base AS kimi
USER root
RUN npm install -g @moonshot-ai/kimi-code@2.4.0 && npm cache clean --force
USER node

# Qwen Code（Apache-2.0），原生 ACP：qwen --acp。
FROM base AS qwen
USER root
RUN npm install -g @qwen-code/qwen-code@0.26.2 && npm cache clean --force
USER node

# Qoder CLI（npm 包里附 Apache-2.0 文本，源码未公开），原生 ACP：qodercli --acp。必须登录 Qoder 账号。
FROM base AS qoder
USER root
RUN npm install -g @qoder-ai/qodercli@1.1.80 && npm cache clean --force
USER node

# OAR（Apache-2.0）加它要驱动的 claude、codex 原生程序：验证插话、审批，以及包成 ACP 适配器的成本。
FROM base AS oar
USER root
RUN npm install -g @anthropic-ai/claude-code@2.1.312 @openai/codex@0.165.0 && npm cache clean --force
COPY --chown=node:node oar/package.json oar/package-lock.json /opt/oar/
RUN cd /opt/oar && npm ci --omit=dev && npm cache clean --force && chown -R node:node /opt/oar
USER node
