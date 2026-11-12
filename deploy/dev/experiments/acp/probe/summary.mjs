#!/usr/bin/env node
// 把 out 目录里各场景的结果汇总成 Markdown 表：node summary.mjs <out 目录>
// 第一张是探针（ACP 客户端）的场景，第二张是 OAR 直接驱动（oar/oar-run.mjs）的结果。
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2] ?? "out";
const reports = new Map();
for (const file of readdirSync(dir)) {
  if (!file.endsWith(".json")) continue;
  const report = JSON.parse(readFileSync(join(dir, file), "utf8"));
  if (!reports.has(report.agent)) reports.set(report.agent, {});
  reports.get(report.agent)[report.scenario] = report;
}

const yes = (value) => (value ? "✅" : "❌");
const failed = (report) => (report ? `❌ ${JSON.stringify(report.error).slice(0, 80)}` : "—");
const brief = (error) => String(error?.message ?? JSON.stringify(error)).slice(0, 60);

const cell = {
  init(report) {
    if (!report?.ok) return failed(report);
    const r = report.result;
    const modes = r.modes?.availableModes?.map((m) => m.id).join("/") ?? "无";
    const auth = (r.authMethods ?? []).map((m) => m.id).join(",") || "无";
    return `${r.agentInfo?.version}；模式 ${modes}（默认 ${r.modes?.currentModeId}）；登录 ${auth}`;
  },
  chat: (report) => (report?.ok ? yes(report.result.continues) : failed(report)),
  load(report) {
    if (!report?.ok) return failed(report);
    const r = report.result;
    if (r.error) return `❌ ${brief(r.error)}`;
    const replayed = (r.replay.user_message_chunk ?? 0) + (r.replay.agent_message_chunk ?? 0);
    return `${yes(r.resumes)} 回放 ${replayed} 条`;
  },
  redo(report) {
    if (!report?.ok) return failed(report);
    const r = report.result;
    return `${r.cancelled.stopReason}（${r.cancelled.stopAfterCancelMs} ms）；历史 ${yes(r.historyKept)}`;
  },
  busy(report) {
    if (!report?.ok) return failed(report);
    const { first, second } = report.result;
    if (!second.ok) return `拒绝第二个：${brief(second.error)}`;
    if (!first.ok) return `第一轮出错：${brief(first.error)}`;
    return report.result.secondAfterFirst ? "排队，前一轮结束后再答" : "并行";
  },
  steer(report) {
    if (!report?.ok) return failed(report);
    const r = report.result;
    if (!r.supported) return "不支持";
    return `${r.steered.outcome ?? brief(r.steered)}；生效 ${yes(r.pineappleAfterSteer)}；空闲时 ${r.idleOutcome.outcome}`;
  },
  permission(report) {
    if (!report?.ok) return failed(report);
    return report.result
      .map((c) => `${c.case ?? c.policy}：${c.requests.length ? "问" : "没问"}/${c.stopReason ?? "错"}/${c.fileCreated ? "写了" : "没写"}`)
      .join("；");
  },
  memory(agentReports) {
    const peaks = Object.entries(agentReports)
      .filter(([scenario]) => scenario !== "sessions")
      .map(([, r]) => r.memory?.peakMiB ?? 0);
    const idle = agentReports.chat?.memory?.idleMiB ?? agentReports.init?.memory?.idleMiB ?? "—";
    const perSession = agentReports.sessions?.result?.perSessionMiB;
    const extra = perSession === undefined ? "" : ` / ${perSession > 5 ? `+${perSession}` : "≈0"}`;
    return `${idle} / ${Math.max(...peaks)}${extra}`;
  },
};

function probeTable() {
  const header = ["agent", "init", "续聊", "load", "resume", "打断重来", "忙时再发提示", "插话扩展"];
  header.push("权限（问否/停止原因/文件）", "内存 MiB 空闲/峰值/每多一个会话");
  const rows = [header];
  for (const [agent, r] of [...reports].sort()) {
    if (r.approval || r.account) continue;
    rows.push([
      agent,
      cell.init(r.init),
      cell.chat(r.chat),
      cell.load(r.load),
      cell.load(r.resume),
      cell.redo(r.redo),
      cell.busy(r.busy),
      cell.steer(r.steer),
      cell.permission(r.permission),
      cell.memory(r),
    ]);
  }
  return rows;
}

function oarTable() {
  const rows = [["OAR 运行时", "插话", "审批", "启动参数", "登录/额度", "内存峰值 MiB"]];
  for (const [agent, r] of [...reports].sort()) {
    if (!r.approval && !r.account) continue;
    const s = r.steer?.result;
    const a = r.approval?.result;
    const acct = r.account?.result;
    rows.push([
      agent,
      s ? `${s.landed}；立即生效 ${yes(s.linesAfterSteer <= 5 && s.pineappleAfterSteer)}` : "—",
      a ? `应答通道 ${a.appRequests.length}；未经确认就执行 ${yes(a.fileCreatedWithoutAsking)}` : "—",
      a ? a.commandLines.join(" ； ").slice(0, 160) : "—",
      acct ? `login ${yes(acct.hasLogin)}；额度 ${acct.accountUsage?.kind ?? "—"}` : "—",
      Math.max(r.steer?.memory?.peakMiB ?? 0, r.approval?.memory?.peakMiB ?? 0),
    ]);
  }
  return rows;
}

for (const rows of [probeTable(), oarTable()]) {
  for (const [i, row] of rows.entries()) {
    console.log(`| ${row.join(" | ")} |`);
    if (i === 0) console.log(`|${row.map(() => " --- ").join("|")}|`);
  }
  console.log();
}
