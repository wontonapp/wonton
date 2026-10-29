// 最小的 ACP 客户端连接：JSON-RPC 2.0，按行分隔的 JSON，走子进程的标准输入输出。
// 不用 SDK，是为了看清线上的原始报文（每条都写进 trace 文件），也方便发扩展方法和并发请求。
import { spawn } from "node:child_process";
import { appendFileSync, readFileSync, readdirSync } from "node:fs";

export class Rpc {
  constructor(command, args, { env, cwd, trace }) {
    this.trace = trace;
    this.nextId = 1;
    this.pending = new Map();
    this.requestHandlers = new Map();
    this.notificationHandlers = new Map();
    this.exited = new Promise((resolve) => (this.resolveExit = resolve));
    this.child = spawn(command, args, { env, cwd, stdio: ["pipe", "pipe", "pipe"] });
    this.child.on("exit", (code, signal) => {
      this.log("exit", { code, signal });
      for (const { reject } of this.pending.values()) reject(new Error(`agent 退出：${code ?? signal}`));
      this.pending.clear();
      this.resolveExit({ code, signal });
    });
    this.child.stderr.on("data", (chunk) => this.log("stderr", chunk.toString()));
    let buffer = "";
    this.child.stdout.on("data", (chunk) => {
      buffer += chunk.toString();
      let newline;
      while ((newline = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, newline).trim();
        buffer = buffer.slice(newline + 1);
        if (line) this.receive(line);
      }
    });
  }

  get pid() {
    return this.child.pid;
  }

  log(kind, payload) {
    if (this.trace) appendFileSync(this.trace, JSON.stringify({ t: Date.now(), kind, payload }) + "\n");
  }

  send(message) {
    this.log("out", message);
    this.child.stdin.write(JSON.stringify(message) + "\n");
  }

  request(method, params) {
    const id = this.nextId++;
    const started = Date.now();
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method, started });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }

  notify(method, params) {
    this.send({ jsonrpc: "2.0", method, params });
  }

  onRequest(method, handler) {
    this.requestHandlers.set(method, handler);
  }

  onNotification(method, handler) {
    this.notificationHandlers.set(method, handler);
  }

  receive(line) {
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      this.log("garbage", line);
      return;
    }
    this.log("in", message);
    if (message.method === undefined) return this.settle(message);
    if (message.id === undefined) {
      this.notificationHandlers.get(message.method)?.(message.params);
      return;
    }
    this.answer(message);
  }

  settle(message) {
    const call = this.pending.get(message.id);
    if (!call) return;
    this.pending.delete(message.id);
    if (message.error) {
      const error = new Error(`${call.method}: ${message.error.message}`);
      error.rpc = message.error;
      call.reject(error);
    } else {
      call.resolve(message.result);
    }
  }

  async answer(message) {
    const handler = this.requestHandlers.get(message.method);
    if (!handler) {
      this.send({ jsonrpc: "2.0", id: message.id, error: { code: -32601, message: `未实现 ${message.method}` } });
      return;
    }
    try {
      const result = await handler(message.params, message.id);
      this.send({ jsonrpc: "2.0", id: message.id, result });
    } catch (error) {
      this.send({ jsonrpc: "2.0", id: message.id, error: { code: -32603, message: String(error.message ?? error) } });
    }
  }

  async close() {
    this.child.stdin.end();
    const timer = setTimeout(() => this.child.kill("SIGTERM"), 3000);
    const killer = setTimeout(() => this.child.kill("SIGKILL"), 8000);
    await this.exited;
    clearTimeout(timer);
    clearTimeout(killer);
  }
}

// agent 进程和它拉起的全部子孙进程的 pid。
export function processTree(rootPid) {
  const children = new Map();
  for (const entry of readdirSync("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const stat = readFileSync(`/proc/${entry}/stat`, "utf8");
      const ppid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
      if (!children.has(ppid)) children.set(ppid, []);
      children.get(ppid).push(Number(entry));
    } catch {
      // 进程在遍历时退出
    }
  }
  const pids = [];
  const stack = [rootPid];
  while (stack.length) {
    const pid = stack.pop();
    pids.push(pid);
    stack.push(...(children.get(pid) ?? []));
  }
  return pids;
}

function readProc(pid, file) {
  try {
    return readFileSync(`/proc/${pid}/${file}`, "utf8");
  } catch {
    return null;
  }
}

// 进程树的常驻内存（KiB）和进程数。读不到的进程（已经退出）跳过。
export function treeRssKiB(rootPid) {
  let rssKiB = 0;
  let processes = 0;
  for (const pid of processTree(rootPid)) {
    const status = readProc(pid, "status");
    if (status === null) continue;
    rssKiB += Number(/VmRSS:\s+(\d+)/.exec(status)?.[1] ?? 0);
    processes += 1;
  }
  return { rssKiB, processes };
}

// 进程树里每个进程的命令行，用来确认实际启动参数（比如有没有 --dangerously-skip-permissions）。
export function treeCommandLines(rootPid) {
  return processTree(rootPid)
    .map((pid) => readProc(pid, "cmdline"))
    .filter((cmdline) => cmdline)
    .map((cmdline) => cmdline.split("\0").filter(Boolean).join(" "));
}
