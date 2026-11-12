#!/usr/bin/env node
// OAR 直接驱动 claude 或 codex，只验证它补 ACP 空白的部分（#43 第二组）：
//   node oar-run.mjs <claude|codex> steer     插话是不是真插话
//   node oar-run.mjs <claude|codex> approval  原生审批请求能不能由我们回答、实际启动参数
//   node oar-run.mjs <claude|codex> account   不经终端的登录、登录状态、额度、模型列表
// 结果以 JSON 打到标准输出，并写进 /out/oar-<runtime>-<场景>.json。
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { awaitIdle, defaultRuntimes, promptAndWait } from "@botiverse/oar";
import { treeCommandLines, treeRssKiB } from "/probe/rpc.mjs";

const LONG_TASK =
  "Write the English words for the numbers from one to three hundred, one per line " +
  "(one, two, three, ...). Do not use any tools, do not skip numbers, no commentary.";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function open(runtimeId, cwd, extra = {}) {
  const runtime = defaultRuntimes.require(runtimeId);
  const installation = await runtime.installation();
  if (installation?.kind !== "available") throw new Error(`${runtimeId} 不可用：${JSON.stringify(installation)}`);
  mkdirSync(cwd, { recursive: true });
  const session = await runtime.session(installation, { cwd, ...extra });
  const events = [];
  session.events((event) => events.push({ at: Date.now(), ...event }));
  return { runtime, installation, session, events };
}

const textOf = (events, from = 0) =>
  events.slice(from).filter((e) => e.kind === "text_delta" && (e.agentPath?.length ?? 1) <= 1).map((e) => e.text).join("");

async function steer(runtimeId) {
  const { session, events } = await open(runtimeId, "/workspace/oar-steer");
  const canSteer = typeof session.steer === "function";
  const started = session.prompt(LONG_TASK);
  for (let i = 0; i < 900 && textOf(events).split("\n").length < 6; i += 1) await sleep(100);
  const mark = events.length;
  const landed = await session.steerOrQueue("Stop counting now. From here on reply with only the word PINEAPPLE.");
  await started;
  const outcome = await awaitIdle(session);
  // 退回排队时插话的那一轮在后面单独跑，再等一次空闲。
  await sleep(500);
  const second = await awaitIdle(session);
  const after = textOf(events, mark);
  const result = {
    canSteer,
    landed: landed.landed,
    accepted: landed.result.kind,
    outcomes: [outcome?.kind, second?.kind],
    turnsEnded: events.filter((e) => e.kind === "turn_ended").length,
    pineappleAfterSteer: after.includes("PINEAPPLE"),
    linesAfterSteer: after.split("\n").filter(Boolean).length,
  };
  await session.dispose();
  return result;
}

async function approval(runtimeId) {
  const cwd = "/workspace/oar-approval";
  const { session, events } = await open(runtimeId, cwd);
  const commandLines = treeCommandLines(process.pid).filter((line) => !line.includes("oar-run.mjs"));
  const run = await promptAndWait(session, "Run this exact shell command with your shell tool: echo hello > oar.txt", {
    timeoutMs: 180_000,
  });
  const appRequests = events.filter((e) => e.kind === "app_request").map((e) => e.type);
  const answered = events.filter((e) => e.kind === "app_answered").length;
  const result = {
    run: run.kind,
    outcome: run.outcome?.kind,
    fileCreatedWithoutAsking: existsSync(join(cwd, "oar.txt")),
    appRequests,
    answeredByOar: answered,
    commandLines,
  };
  // claude 的权限模式能不能由宿主指定（DeepSeek harness 为此给 OAR 打了补丁）。
  if (runtimeId === "claude") {
    result.permissionModeLaunchArg = await open(runtimeId, cwd, { launchArgs: ["--permission-mode", "default"] }).then(
      async ({ session: s }) => {
        await s.dispose();
        return "接受";
      },
      (error) => `拒绝：${error.message}`,
    );
  }
  await session.dispose();
  return result;
}

async function account(runtimeId) {
  const runtime = defaultRuntimes.require(runtimeId);
  const installation = await runtime.installation();
  const call = async (name) => {
    if (typeof runtime[name] !== "function") return "没有这个接口";
    return runtime[name](installation).catch((error) => ({ error: error.message }));
  };
  const models = await call("listModels");
  return {
    installation: installation?.kind,
    hasLogin: typeof runtime.login === "function",
    authStatus: await call("authStatus"),
    accountUsage: await call("accountUsage"),
    models: models?.kind === "ok" ? { kind: "ok", count: models.models?.length } : models,
  };
}

const SCENARIOS = { steer, approval, account };

async function main() {
  const [runtimeId, scenario] = process.argv.slice(2);
  const started = Date.now();
  const sampler = { peakKiB: 0 };
  const timer = setInterval(() => {
    sampler.peakKiB = Math.max(sampler.peakKiB, treeRssKiB(process.pid).rssKiB);
  }, 500);
  let result;
  let error;
  try {
    result = await SCENARIOS[scenario](runtimeId);
  } catch (e) {
    error = e.message;
  }
  clearInterval(timer);
  const report = {
    agent: `oar-${runtimeId}`,
    scenario,
    ok: error === undefined,
    error,
    ms: Date.now() - started,
    memory: { peakMiB: Math.round(sampler.peakKiB / 1024), hostProcessMiB: Math.round(process.memoryUsage().rss / 1048576) },
    result,
  };
  mkdirSync("/out", { recursive: true });
  writeFileSync(`/out/oar-${runtimeId}-${scenario}.json`, JSON.stringify(report, null, 2));
  process.stdout.write(JSON.stringify(report) + "\n");
  process.exit(0);
}

main();
