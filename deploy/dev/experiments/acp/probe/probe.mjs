#!/usr/bin/env node
// 像编辑器一样驱动一个 ACP agent，跑 #43 的验证场景，结果以 JSON 打到标准输出。
//   node probe.mjs <场景> [--name <名字>] [--out <目录>] [--state <文件>] [--mode <id>]
//                  [--auth <methodId>] -- <agent 命令> [参数...]
// --auth：initialize 之后先 authenticate（比如 Qoder 的 qodercli-login）
// 场景：init chat load resume redo busy steer permission sessions login（load、resume 恢复 chat 建的会话）
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { Rpc, treeCommandLines, treeRssKiB } from "./rpc.mjs";

const CODE_WORD = "tangerine-42";
const LONG_TASK =
  "Write the English words for the numbers from one to three hundred, one per line " +
  "(one, two, three, ...). Do not use any tools, do not skip numbers, no commentary.";
const TURN_TIMEOUT_MS = Number(process.env.PROBE_TURN_TIMEOUT_MS ?? 240_000);

function parseArgs(argv) {
  const split = argv.indexOf("--");
  if (split < 0 || split === argv.length - 1) throw new Error("缺少 -- <agent 命令>");
  const [scenario, ...flags] = argv.slice(0, split);
  const options = { scenario, name: "agent", out: "/out", state: "/state/session.json" };
  for (let i = 0; i < flags.length; i += 2) {
    const key = flags[i].replace(/^--/, "").replace(/-(\w)/g, (_, c) => c.toUpperCase());
    options[key] = flags[i + 1];
  }
  options.command = argv[split + 1];
  options.args = argv.slice(split + 2);
  return options;
}

function withTimeout(promise, ms, label) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} 超时（${ms} ms）`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// 一次连接的全部状态：收到的更新、权限请求、内存采样。
class Context {
  constructor(options) {
    this.options = options;
    mkdirSync(options.out, { recursive: true });
    this.trace = join(options.out, `${options.name}-${options.scenario}.trace.jsonl`);
    writeFileSync(this.trace, "");
    this.updates = [];
    this.permissions = [];
    this.policy = "reject";
    this.memory = { idleKiB: null, peakKiB: 0, processes: 0 };
  }

  connect() {
    const { command, args } = this.options;
    this.rpc = new Rpc(command, args, { env: process.env, cwd: process.cwd(), trace: this.trace });
    this.rpc.onNotification("session/update", (params) => this.updates.push({ at: Date.now(), ...params }));
    this.rpc.onRequest("session/request_permission", (params) => this.answerPermission(params));
    this.elicitations = [];
    this.rpc.onRequest("elicitation/create", async (params) => {
      this.elicitations.push({ mode: params.mode, url: params.url, message: params.message });
      return { action: "decline" };
    });
    this.sampler = setInterval(() => this.sample(), 500);
    return this.rpc;
  }

  sample() {
    const { rssKiB, processes } = treeRssKiB(this.rpc.pid);
    if (rssKiB > this.memory.peakKiB) this.memory.peakKiB = rssKiB;
    this.memory.processes = Math.max(this.memory.processes, processes);
    return rssKiB;
  }

  markIdle() {
    this.memory.idleKiB = this.sample();
  }

  async answerPermission(params) {
    const record = {
      at: Date.now(),
      toolCall: {
        title: params.toolCall?.title,
        kind: params.toolCall?.kind,
        status: params.toolCall?.status,
        rawInput: params.toolCall?.rawInput,
        hasDiff: (params.toolCall?.content ?? []).some((c) => c.type === "diff"),
        locations: params.toolCall?.locations,
      },
      options: (params.options ?? []).map((o) => ({ id: o.optionId, name: o.name, kind: o.kind })),
      policy: this.policy,
    };
    this.permissions.push(record);
    if (this.policy === "cancel") {
      await sleep(500);
      this.rpc.notify("session/cancel", { sessionId: params.sessionId });
      record.answer = "cancelled";
      return { outcome: { outcome: "cancelled" } };
    }
    const wanted = this.policy === "allow" ? ["allow_once", "allow_always"] : ["reject_once", "reject_always"];
    const option = wanted.map((kind) => params.options.find((o) => o.kind === kind)).find(Boolean);
    if (!option) {
      record.answer = "cancelled（没有对应的选项）";
      return { outcome: { outcome: "cancelled" } };
    }
    record.answer = option.optionId;
    return { outcome: { outcome: "selected", optionId: option.optionId } };
  }

  textSince(mark, sessionId) {
    return this.updates
      .slice(mark)
      .filter((u) => u.sessionId === sessionId && u.update.sessionUpdate === "agent_message_chunk")
      .map((u) => (u.update.content?.type === "text" ? u.update.content.text : ""))
      .join("");
  }

  kindsSince(mark, sessionId) {
    const counts = {};
    for (const u of this.updates.slice(mark)) {
      if (u.sessionId !== sessionId) continue;
      counts[u.update.sessionUpdate] = (counts[u.update.sessionUpdate] ?? 0) + 1;
    }
    return counts;
  }

  chunksSince(mark, sessionId) {
    return this.kindsSince(mark, sessionId).agent_message_chunk ?? 0;
  }

  async close() {
    clearInterval(this.sampler);
    await this.rpc?.close();
  }
}

// 桥不声明 fs、terminal：agent 用自己容器里的文件和终端。init 场景另外声明登录相关的能力
// （终端登录、URL 引导、自定义模型网关），只为看清各家提供哪些登录方式。
async function initialize(ctx, extraCapabilities = {}) {
  const rpc = ctx.connect();
  const init = await withTimeout(
    rpc.request("initialize", {
      protocolVersion: 1,
      clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false, ...extraCapabilities },
      clientInfo: { name: "wonton-acp-probe", version: "0.0.0" },
    }),
    60_000,
    "initialize",
  );
  if (ctx.options.auth) {
    ctx.auth = await withTimeout(rpc.request("authenticate", { methodId: ctx.options.auth }), 60_000, "authenticate");
  }
  return init;
}

async function newSession(ctx, cwd) {
  mkdirSync(cwd, { recursive: true });
  const session = await withTimeout(
    ctx.rpc.request("session/new", { cwd, mcpServers: [] }),
    120_000,
    "session/new",
  );
  if (ctx.options.mode) {
    await ctx.rpc.request("session/set_mode", { sessionId: session.sessionId, modeId: ctx.options.mode });
  }
  ctx.markIdle();
  return session;
}

// 发一轮提示并等它结束；返回停止原因、这一轮的回复文字、耗时。
async function turn(ctx, sessionId, text) {
  const mark = ctx.updates.length;
  const started = Date.now();
  const response = await withTimeout(
    ctx.rpc.request("session/prompt", { sessionId, prompt: [{ type: "text", text }] }),
    TURN_TIMEOUT_MS,
    "session/prompt",
  );
  await sleep(200);
  return {
    stopReason: response.stopReason,
    text: ctx.textSince(mark, sessionId).trim().slice(-400),
    updates: ctx.kindsSince(mark, sessionId),
    ms: Date.now() - started,
  };
}

// 发出提示但不等待，返回 promise 和这一轮开始时的更新位置。
function startTurn(ctx, sessionId, text) {
  const mark = ctx.updates.length;
  const started = Date.now();
  const promise = withTimeout(
    ctx.rpc.request("session/prompt", { sessionId, prompt: [{ type: "text", text }] }),
    TURN_TIMEOUT_MS,
    "session/prompt",
  ).then(
    (response) => ({ ok: true, stopReason: response.stopReason, ms: Date.now() - started, at: Date.now() }),
    (error) => ({ ok: false, error: error.rpc ?? error.message, ms: Date.now() - started, at: Date.now() }),
  );
  return { mark, started, promise };
}

async function waitForChunks(ctx, sessionId, mark, count, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (ctx.chunksSince(mark, sessionId) >= count) return true;
    await sleep(100);
  }
  return false;
}

const remembers = (text) => text.toLowerCase().includes(CODE_WORD);

const AUTH_DISCOVERY = { auth: { terminal: true, _meta: { gateway: true } }, elicitation: { url: {} } };

async function scenarioInit(ctx) {
  const init = await initialize(ctx, AUTH_DISCOVERY);
  const session = await newSession(ctx, "/workspace/init");
  const result = {
    protocolVersion: init.protocolVersion,
    agentInfo: init.agentInfo,
    agentCapabilities: init.agentCapabilities,
    authMethods: init.authMethods,
    initMetaKeys: Object.keys(init._meta ?? {}),
    steering: init._meta?.steering ?? null,
    modes: session.modes,
    configOptions: (session.configOptions ?? []).map((o) => ({
      id: o.id,
      current: o.currentValue,
      values: (o.options ?? []).flatMap((v) => (v.options ? v.options : [v])).map((v) => v.value),
    })),
    models: session.models
      ? { current: session.models.currentModelId, count: session.models.availableModels?.length }
      : undefined,
  };
  if (init.agentCapabilities?.providers) {
    result.providers = await ctx.rpc.request("providers/list", {}).catch((e) => ({ error: e.rpc ?? e.message }));
  }
  return result;
}

async function scenarioChat(ctx) {
  const init = await initialize(ctx);
  const cwd = "/workspace/chat";
  const { sessionId } = await newSession(ctx, cwd);
  const first = await turn(ctx, sessionId, `Remember this code word: ${CODE_WORD}. Reply with only OK.`);
  const second = await turn(ctx, sessionId, "What is the code word I gave you? Reply with only the code word.");
  mkdirSync(join(ctx.options.state, ".."), { recursive: true });
  writeFileSync(ctx.options.state, JSON.stringify({ sessionId, cwd }));
  return {
    loadSession: init.agentCapabilities?.loadSession ?? false,
    sessionCapabilities: init.agentCapabilities?.sessionCapabilities,
    sessionId,
    first,
    second,
    continues: remembers(second.text),
  };
}

async function scenarioLoad(ctx) {
  const { sessionId, cwd } = JSON.parse(readFileSync(ctx.options.state, "utf8"));
  const init = await initialize(ctx);
  const method = ctx.options.scenario === "resume" ? "session/resume" : "session/load";
  const mark = ctx.updates.length;
  const started = Date.now();
  let loaded;
  try {
    loaded = await withTimeout(ctx.rpc.request(method, { sessionId, cwd, mcpServers: [] }), 120_000, method);
  } catch (error) {
    return { method, capability: init.agentCapabilities, error: error.rpc ?? error.message };
  }
  ctx.markIdle();
  const replay = ctx.kindsSince(mark, sessionId);
  const recall = await turn(ctx, sessionId, "What is the code word I gave you? Reply with only the code word.");
  let listed;
  if (init.agentCapabilities?.sessionCapabilities?.list) {
    const list = await ctx.rpc.request("session/list", { cwd }).catch((e) => ({ error: e.message }));
    listed = (list.sessions ?? []).some((s) => s.sessionId === sessionId);
  }
  return {
    method,
    loadMs: Date.now() - started - recall.ms,
    loadedKeys: Object.keys(loaded ?? {}),
    replay,
    recall,
    resumes: remembers(recall.text),
    listed,
  };
}

// 打断重来：长任务进行中 session/cancel，再带上新消息重新提示，看历史是否还在。
async function scenarioRedo(ctx) {
  await initialize(ctx);
  const { sessionId } = await newSession(ctx, "/workspace/redo");
  const first = await turn(ctx, sessionId, `Remember this code word: ${CODE_WORD}. Reply with only OK.`);
  const long = startTurn(ctx, sessionId, LONG_TASK);
  const streaming = await waitForChunks(ctx, sessionId, long.mark, 5);
  const cancelAt = Date.now();
  ctx.rpc.notify("session/cancel", { sessionId });
  const cancelled = await long.promise;
  const partial = ctx.textSince(long.mark, sessionId);
  const redo = await turn(
    ctx,
    sessionId,
    "Stop the previous task. New request: reply with only the code word I gave you at the start.",
  );
  return {
    first: first.stopReason,
    streamingBeforeCancel: streaming,
    cancelled: { ...cancelled, stopAfterCancelMs: cancelled.at - cancelAt },
    partialLines: partial.split("\n").filter(Boolean).length,
    lateChunks: ctx.updates.filter(
      (u) => u.sessionId === sessionId && u.at > cancelled.at && u.at < cancelled.at + 1000 && u.update.sessionUpdate === "agent_message_chunk",
    ).length,
    redo,
    historyKept: remembers(redo.text),
  };
}

// 不用扩展时，在一轮进行中再发一个 session/prompt，看 agent 怎么处理。
async function scenarioBusy(ctx) {
  await initialize(ctx);
  const { sessionId } = await newSession(ctx, "/workspace/busy");
  const long = startTurn(ctx, sessionId, LONG_TASK);
  const streaming = await waitForChunks(ctx, sessionId, long.mark, 5);
  const second = startTurn(ctx, sessionId, "Reply with only the word PINEAPPLE.");
  const [firstDone, secondDone] = await Promise.all([long.promise, second.promise]);
  const text = ctx.textSince(long.mark, sessionId);
  return {
    streamingBeforeSecond: streaming,
    first: firstDone,
    second: secondDone,
    secondAfterFirst: secondDone.at >= firstDone.at,
    sawPineapple: text.includes("PINEAPPLE"),
    numberLines: text.split("\n").filter((l) => /^[a-z -]+$/i.test(l.trim()) && l.trim() !== "PINEAPPLE").length,
  };
}

// 真插话：_session/steering 扩展（claude-agent-acp、codex-acp 的约定）。
async function scenarioSteer(ctx) {
  const init = await initialize(ctx);
  if (!init._meta?.steering?.supported) return { supported: false };
  const { sessionId } = await newSession(ctx, "/workspace/steer");
  const idle = await ctx.rpc
    .request("_session/steering", {
      sessionId,
      prompt: [{ type: "text", text: "Reply with only OK." }],
      _meta: { steering: { idleBehavior: "promptRequired" } },
    })
    .catch((e) => ({ error: e.rpc ?? e.message }));
  const long = startTurn(ctx, sessionId, LONG_TASK);
  const streaming = await waitForChunks(ctx, sessionId, long.mark, 5);
  const steerMark = ctx.updates.length;
  const steered = await ctx.rpc
    .request("_session/steering", {
      sessionId,
      prompt: [{ type: "text", text: "Stop counting now. From here on reply with only the word PINEAPPLE." }],
      _meta: { steering: { idleBehavior: "promptRequired" } },
    })
    .catch((e) => ({ error: e.rpc ?? e.message }));
  const done = await long.promise;
  const afterSteer = ctx.textSince(steerMark, sessionId);
  return {
    supported: true,
    idleOutcome: idle,
    streamingBeforeSteer: streaming,
    steered,
    turn: done,
    userMessageEchoes: ctx.kindsSince(steerMark, sessionId).user_message_chunk ?? 0,
    pineappleAfterSteer: afterSteer.includes("PINEAPPLE"),
    linesAfterSteer: afterSteer.split("\n").filter(Boolean).length,
    tail: afterSteer.trim().slice(-200),
  };
}

const EDIT = (file) =>
  `Use your file writing tool to create a file named ${file} in the current directory containing exactly: hello`;
const SHELL = (file) => `Run this exact shell command with your shell tool and no other tool: echo hello > ${file}`;
const PERMISSION_CASES = [
  { id: "allow-edit", policy: "allow", file: "allow.txt", prompt: EDIT("allow.txt") },
  { id: "reject-shell", policy: "reject", file: "reject.txt", prompt: SHELL("reject.txt") },
  { id: "cancel-edit", policy: "cancel", file: "cancel.txt", prompt: EDIT("cancel.txt") },
  { id: "allow-shell", policy: "allow", file: "shell.txt", prompt: SHELL("shell.txt") },
];

// 权限请求：同一个模式下，分别回答允许（改文件、跑命令）、拒绝、取消。
async function scenarioPermission(ctx) {
  await initialize(ctx);
  const results = [];
  const only = process.env.PROBE_CASES?.split(",");
  for (const testCase of PERMISSION_CASES.filter((c) => !only || only.includes(c.id))) {
    const cwd = `/workspace/permission-${testCase.id}`;
    rmSync(join(cwd, testCase.file), { force: true });
    const { sessionId, modes } = await newSession(ctx, cwd);
    ctx.policy = testCase.policy;
    const before = ctx.permissions.length;
    let outcome;
    try {
      outcome = await turn(ctx, sessionId, testCase.prompt);
    } catch (error) {
      outcome = { error: error.rpc ?? error.message };
    }
    results.push({
      case: testCase.id,
      policy: testCase.policy,
      mode: ctx.options.mode ?? modes?.currentModeId,
      requests: ctx.permissions.slice(before),
      stopReason: outcome.stopReason,
      error: outcome.error,
      text: outcome.text?.slice(-200),
      fileCreated: existsSync(join(cwd, testCase.file)),
    });
  }
  return results;
}

// 不经终端登录：用 ACP 的 authenticate 发起登录，agent 经 elicitation/create（URL 模式）把链接和设备码交给客户端，
// 桥可以原样转进聊天。这里只看拿不拿得到链接和设备码，随即拒绝，不完成登录。
async function scenarioLogin(ctx) {
  const init = await initialize(ctx, { elicitation: { url: {} } });
  const methodId = init.authMethods?.find((m) => /device/.test(m.id))?.id;
  if (!methodId) return { authMethods: init.authMethods, error: "没有设备码登录方式" };
  const outcome = await withTimeout(ctx.rpc.request("authenticate", { methodId }), 60_000, "authenticate").then(
    (result) => ({ ok: true, result }),
    (error) => ({ ok: false, error: error.rpc ?? error.message }),
  );
  return { methodId, elicitations: ctx.elicitations, outcome };
}

// 一个 agent 进程里开多个会话（一个房间一个会话）时的内存：每开一个会话、答一句后采样一次。
async function scenarioSessions(ctx) {
  await initialize(ctx);
  const samples = [{ sessions: 0, rssMiB: Math.round(ctx.sample() / 1024) }];
  for (let i = 1; i <= Number(process.env.PROBE_SESSIONS ?? 5); i += 1) {
    const { sessionId } = await newSession(ctx, `/workspace/sessions-${i}`);
    await turn(ctx, sessionId, "Reply with only OK.");
    await sleep(1000);
    const { processes } = treeRssKiB(ctx.rpc.pid);
    samples.push({ sessions: i, rssMiB: Math.round(ctx.sample() / 1024), processes });
  }
  const last = samples.at(-1);
  return { samples, perSessionMiB: Math.round((last.rssMiB - samples[1].rssMiB) / (samples.length - 2)) };
}

const SCENARIOS = {
  init: scenarioInit,
  chat: scenarioChat,
  load: scenarioLoad,
  resume: scenarioLoad,
  redo: scenarioRedo,
  busy: scenarioBusy,
  steer: scenarioSteer,
  permission: scenarioPermission,
  sessions: scenarioSessions,
  login: scenarioLogin,
};

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const run = SCENARIOS[options.scenario];
  if (!run) throw new Error(`未知场景 ${options.scenario}`);
  const ctx = new Context(options);
  const started = Date.now();
  let result;
  let error;
  try {
    result = await run(ctx);
    ctx.commandLines = ctx.rpc ? treeCommandLines(ctx.rpc.pid) : [];
  } catch (e) {
    error = e.rpc ?? e.message;
  } finally {
    await ctx.close().catch(() => {});
  }
  const report = {
    agent: options.name,
    scenario: options.scenario,
    ok: error === undefined,
    error,
    ms: Date.now() - started,
    memory: { ...ctx.memory, idleMiB: Math.round((ctx.memory.idleKiB ?? 0) / 1024), peakMiB: Math.round(ctx.memory.peakKiB / 1024) },
    commandLines: ctx.commandLines,
    result,
  };
  writeFileSync(join(options.out, `${options.name}-${options.scenario}.json`), JSON.stringify(report, null, 2));
  process.stdout.write(JSON.stringify(report) + "\n");
  process.exit(error === undefined ? 0 : 1);
}

main().catch((error) => {
  console.error(error);
  process.exit(2);
});
