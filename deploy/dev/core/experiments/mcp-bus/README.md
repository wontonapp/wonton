# MCP 总线选型实验

结论见 ADR-0006“验证结果（M0-6）”。三个候选都连同一个假插件 MCP 服务器 `photos`
（streamable HTTP，`search_photos` 普通，`delete_photo` 敏感），用同一个模拟 Bot 客户端 `bot` 调用。
这是独立的 Cargo 工作区，不进入主工作区和 CI。

| 程序 | 作用 |
| --- | --- |
| `src/bin/photos.rs` | 假插件 MCP 服务器，默认监听 `0.0.0.0:8101/mcp` |
| `src/bin/bus.rs` | rmcp 最小总线：Bot 令牌 → 白名单，敏感工具等主人确认，审计写 `audit.jsonl` |
| `src/bin/bot.rs` | 模拟 Bot：`bot <令牌> list`、`bot <令牌> call <工具> '<JSON>'`，地址用 `MCP_URL` 指定 |

## rmcp 最小聚合

```bash
deploy/dev/core/experiments/mcp-bus/run.sh
```

访客 Bot 只看到 `photos__search_photos`，调 `photos__delete_photo` 被拒；主人助手的删除请求挂起，
用 `POST /approvals/<id>?approve=true` 确认后才转发；每一步写一行审计。

## Docker MCP Gateway（v0.44.3）

```bash
cargo build --manifest-path deploy/dev/core/experiments/mcp-bus/Cargo.toml --bins
deploy/dev/core/experiments/mcp-bus/target/debug/photos &
cat >/tmp/photos-catalog.yaml <<'EOF'
registry:
  photos:
    remote:
      url: "http://host.docker.internal:8101/mcp"
      transport_type: http
EOF
# 目录文件必须在 ~/.docker/mcp/catalogs 下；不挂 Docker socket 会启动失败；http 地址要额外放行。
docker run -d --name exp-mcpgw -p 127.0.0.1:8811:8811 \
  -e MCP_GATEWAY_AUTH_TOKEN=gw-shared-token -e DOCKER_MCP_ALLOW_INSECURE_REMOTE_URLS=1 \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /tmp/photos-catalog.yaml:/root/.docker/mcp/catalogs/photos.yaml:ro \
  docker/mcp-gateway:latest --catalog=/root/.docker/mcp/catalogs/photos.yaml --servers=photos \
  --transport=streaming --port=8811 --verify-signatures=false '--interceptor=before:exec:cat >&2'
MCP_URL=http://127.0.0.1:8811/mcp deploy/dev/core/experiments/mcp-bus/target/debug/bot gw-shared-token list
MCP_URL=http://127.0.0.1:8811/mcp deploy/dev/core/experiments/mcp-bus/target/debug/bot gw-shared-token \
  call search_photos '{"query":"海边"}'   # 带拦截器时调用失败：marshalling request
```

不带 `--interceptor`、改用 `--tools=search_photos` 时调用成功，但白名单对所有客户端生效，且只认
`MCP_GATEWAY_AUTH_TOKEN` 这一个令牌。

## IBM ContextForge（v1.1.0）

```bash
docker run -d --name exp-cf -p 127.0.0.1:4444:4444 -e HOST=0.0.0.0 -e GUNICORN_WORKERS=1 \
  -e JWT_SECRET_KEY=<随机> -e AUTH_ENCRYPTION_SECRET=<随机> -e BASIC_AUTH_PASSWORD=<随机> \
  -e PLATFORM_ADMIN_EMAIL=admin@example.com -e PLATFORM_ADMIN_PASSWORD=<随机> \
  -e DEFAULT_USER_PASSWORD=<随机> -e DATABASE_URL=sqlite:////tmp/mcp.db \
  -e MCPGATEWAY_UI_ENABLED=false -e SSRF_ALLOW_PRIVATE_NETWORKS=true \
  ghcr.io/ibm/mcp-context-forge:v1.1.0
TOKEN=$(docker exec exp-cf python -m mcpgateway.utils.create_jwt_token \
  --username admin@example.com --exp 60 --secret <JWT_SECRET_KEY>)
curl -X POST http://127.0.0.1:4444/gateways -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"name":"photos","url":"http://host.docker.internal:8101/mcp","transport":"STREAMABLEHTTP"}'
curl http://127.0.0.1:4444/tools -H "Authorization: Bearer $TOKEN"        # 取两个工具的 id
curl -X POST http://127.0.0.1:4444/servers -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"server":{"name":"guest-bot","associated_tools":["<search 的 id>"]}}'
curl -X POST http://127.0.0.1:4444/tokens -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"name":"guest-bot","expires_in_days":1,"scope":{"server_id":"<虚拟服务器 id>","permissions":["tools.read","tools.execute"]}}'
MCP_URL=http://127.0.0.1:4444/servers/<虚拟服务器 id>/mcp \
  deploy/dev/core/experiments/mcp-bus/target/debug/bot <限定令牌> list
```

限定到访客虚拟服务器的令牌访问别的虚拟服务器会被拒。确认流程没有内置插件；内存用
`docker stats exp-cf` 查看。
