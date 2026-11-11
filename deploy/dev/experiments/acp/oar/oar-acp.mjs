#!/usr/bin/env node
// 原型：把 OAR 包成一个说 ACP 的适配器（#43 第三组）。node oar-acp.mjs <OAR 运行时 id>
// 只覆盖桥要用的部分：initialize、session/new、session/resume、session/prompt、session/cancel，
// 以及插话扩展 _session/steering（与 claude-agent-acp、codex-acp 同一约定）。
// OAR 没有审批的应答通道，所以这里发不出 session/request_permission：这正是要验证的缺口。
import { createInterface } from "node:readline";
import { defaultRuntimes, promptAndWait } from "@botiverse/oar";

const runtimeId = process.argv[2];
const runtime = defaultRuntimes.require(runtimeId);
const sessions = new Map();
let installation;

const send = (message) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
const notify = (sessionId, update) => send({ method: "session/update", params: { sessionId, update } });

// OAR 事件 → ACP session/update。只转根 Agent 的事件，子 Agent 默认不发（ADR-0007）。
function forward(sessionId, event) {
  if ((event.agentPath?.length ?? 1) > 1) return;
  switch (event.kind) {
    case "text_delta":
      return notify(sessionId, { sessionUpdate: "agent_message_chunk", content: { type: "text", text: event.text } });
    case "reasoning":
      return notify(sessionId, {
        sessionUpdate: "agent_thought_chunk",
        content: { type: "text", text: event.content?.text ?? "" },
      });
    case "tool_call_started":
      return notify(sessionId, {
        sessionUpdate: "tool_call",
        toolCallId: event.callId,
        title: event.tool,
        kind: "other",
        status: "in_progress",
      });
    case "tool_call_ended":
      return notify(sessionId, {
        sessionUpdate: "tool_call_update",
        toolCallId: event.callId,
        status: event.result === "failed" ? "failed" : "completed",
      });
    default:
  }
}

async function openSession({ cwd, resume }) {
  installation ??= await runtime.installation();
  if (installation?.kind !== "available") throw new Error(`${runtimeId} 不可用`);
  const session = await runtime.session(installation, resume ? { cwd, resume } : { cwd });
  const entry = { session, busy: null };
  sessions.set(session.id, entry);
  session.events((event) => forward(session.id, event));
  return session.id;
}

const STOP_REASONS = { completed: "end_turn", aborted: "cancelled" };

// ACP v1 一个会话同时只有一轮：忙时先等上一轮结束（桥默认排队）。
async function prompt({ sessionId, prompt: blocks }) {
  const entry = sessions.get(sessionId);
  if (!entry) throw new Error("没有这个会话");
  const text = blocks.filter((b) => b.type === "text").map((b) => b.text).join("\n");
  while (entry.busy) await entry.busy.catch(() => {});
  entry.busy = promptAndWait(entry.session, text);
  try {
    const run = await entry.busy;
    if (run.kind === "rejected") throw new Error(`OAR 拒绝：${run.reason}`);
    const stopReason = STOP_REASONS[run.outcome.kind];
    if (!stopReason) throw new Error(`这一轮失败：${JSON.stringify(run.outcome)}`);
    return { stopReason };
  } finally {
    entry.busy = null;
  }
}

async function steer({ sessionId, prompt: blocks, _meta }) {
  const entry = sessions.get(sessionId);
  const text = blocks.filter((b) => b.type === "text").map((b) => b.text).join("\n");
  if (!entry.busy) {
    if (_meta?.steering?.idleBehavior === "promptRequired") return { outcome: "promptRequired", reason: "noRunningTurn" };
    prompt({ sessionId, prompt: blocks }).catch(() => {});
    return { outcome: "startedNewTurn" };
  }
  const landed = await entry.session.steerOrQueue(text);
  if (landed.result.kind !== "accepted") return { outcome: "failed" };
  return { outcome: landed.landed === "steered" ? "injected" : "queued" };
}

const canSteer = runtimeId === "claude" || runtimeId === "codex" || runtimeId === "qoder";

const HANDLERS = {
  initialize: async () => ({
    protocolVersion: 1,
    agentInfo: { name: "oar-acp", title: `OAR ${runtimeId}`, version: "0.0.0" },
    agentCapabilities: { loadSession: false, sessionCapabilities: { resume: {} }, promptCapabilities: {} },
    authMethods: [],
    _meta: { steering: { supported: canSteer } },
  }),
  "session/new": async (params) => ({ sessionId: await openSession(params) }),
  "session/resume": async (params) => {
    await openSession({ cwd: params.cwd, resume: params.sessionId });
    return {};
  },
  "session/prompt": prompt,
  "_session/steering": steer,
};

async function handle(message) {
  if (message.method === "session/cancel") {
    const entry = sessions.get(message.params.sessionId);
    if (entry?.busy) await entry.session.abort();
    return;
  }
  const handler = HANDLERS[message.method];
  if (message.id === undefined) return;
  if (!handler) return send({ id: message.id, error: { code: -32601, message: `未实现 ${message.method}` } });
  try {
    send({ id: message.id, result: await handler(message.params ?? {}) });
  } catch (error) {
    send({ id: message.id, error: { code: -32603, message: error.message } });
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  if (line.trim()) handle(JSON.parse(line));
});
process.stdin.on("end", async () => {
  for (const { session } of sessions.values()) await session.dispose().catch(() => {});
  process.exit(0);
});
