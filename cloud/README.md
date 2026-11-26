# 云端服务

我们运营的云端服务的部署配置：iroh-relay、iroh-dns-server、推送中继（gorush）、插件目录与节点列表的
发布。这些服务只转发，不存用户账号和业务数据。见 `docs/adr/0012-updates-release-cloud.md`。

- `iroh-relay/`：打洞失败时转发端到端加密 QUIC 流量的中继（nginx 终结 TLS，systemd，无 Docker）。
