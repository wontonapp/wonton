#!/usr/bin/env node
// TCP 转发：node relay.mjs <监听端口> <目标主机> <目标端口>。
// 模拟“插件网络是 internal，只经核心的别名连得上模型网关”（插件规范的 network.egress）。
import { createConnection, createServer } from "node:net";

const [listenPort, targetHost, targetPort] = process.argv.slice(2);
createServer((client) => {
  const upstream = createConnection({ host: targetHost, port: Number(targetPort) });
  client.pipe(upstream).pipe(client);
  const close = () => {
    client.destroy();
    upstream.destroy();
  };
  client.on("error", close);
  upstream.on("error", close);
}).listen(Number(listenPort), "0.0.0.0");
