// 用官方 matrix-widget-api 扮演 widget 页：页面跑在 App 的 WebView 主框架里，没有父窗口，
// postMessage 发给自己，由注入脚本截走交给核心；核心经订阅注入合成的 MessageEvent。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { runInThisContext } from "node:vm";
import { format } from "node:util";

const emit = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
// matrix-widget-api 把每条报文都 console.log 一遍，只留警告和错误。
for (const method of ["log", "debug", "info"]) console[method] = () => {};
for (const method of ["warn", "error"]) {
  console[method] = (...args) => process.stderr.write(`${format(...args)}\n`);
}
function fail(error) {
  console.error(error);
  process.exit(1);
}
process.on("uncaughtException", fail);
process.on("unhandledRejection", fail);

// Node's MessageEvent restricts source to MessagePort; browsers accept Window.
class WindowMessageEvent extends Event {
  constructor(type, { data = null, origin = "", source = null } = {}) {
    super(type);
    this.data = data;
    this.origin = origin;
    this.source = source;
  }
}
class FakeWindow extends EventTarget {
  postMessage(data, targetOrigin = "*") {
    assert.ok(targetOrigin === "*" || targetOrigin === this.location.origin, targetOrigin);
    const copy = structuredClone(data);
    setImmediate(() => this.dispatchEvent(new WindowMessageEvent("message", {
      data: copy, origin: this.location.origin, source: this,
    })));
  }
}

function installWindow(url) {
  const window = new FakeWindow();
  const document = new EventTarget();
  // 主题变量在 widget 页上无关紧要：给一个现成的根元素，注入脚本直接写上。
  document.documentElement = { style: { setProperty() {} } };
  window.parent = window;
  window.self = window;
  window.window = window;
  window.document = document;
  window.location = new URL(url);
  window.MessageEvent = WindowMessageEvent;
  window.webkit = { messageHandlers: { wontonBridge: { postMessage(message) {
    assert.equal(typeof message, "string");
    emit({ message, origin: window.location.origin });
  } } } };
  // UMD 包把 mxwidgets 挂在 window 上；matrix-widget-api 用 globalThis 当自己的窗口（globalThis.parent、globalThis.addEventListener）。
  Object.assign(globalThis, {
    window, document, self: window, parent: window, location: window.location,
    origin: window.location.origin, MessageEvent: WindowMessageEvent,
    addEventListener: window.addEventListener.bind(window),
    removeEventListener: window.removeEventListener.bind(window),
    dispatchEvent: window.dispatchEvent.bind(window),
    postMessage: window.postMessage.bind(window),
  });
}
const inject = (script) => runInThisContext(script, { filename: "native-injection.js", timeout: 1000 });

let api;
const received = [];
let busy = false;
let exiting = false;

async function initialize(input) {
  installWindow(input.url);
  inject(input.script);
  runInThisContext(
    readFileSync(new URL("./generated/matrix-widget-api-1.20.0.min.js", import.meta.url), "utf8"),
    { filename: "matrix-widget-api.min.js" },
  );
  const query = new URL(input.url).searchParams;
  const parent = new URL(query.get("parentUrl"));
  // 1.20.0 的压缩包（tinyify）挂出来的是模块的懒加载函数，调一次才拿到导出。
  const { WidgetApi } = globalThis.window.mxwidgets();
  api = new WidgetApi(query.get("widgetId"), parent.origin);
  api.requestCapabilities(input.capabilities);
  api.on("action:send_to_device", (event) => {
    event.preventDefault();
    received.push(event.detail.data);
    api.transport.reply(event.detail, {});
  });
  const ready = new Promise((resolve) => api.once("ready", resolve));
  api.start();
  await api.sendContentLoaded();
  await ready;
  emit({
    ready: true,
    approved: api.approvedCapabilities ?? null,
    query: Object.fromEntries(query),
  });
}

const calls = {
  sendState: (p) => api.sendStateEvent(p.type, p.stateKey, p.content),
  openId: () => api.requestOpenIDConnectToken(),
  sendToDevice: (p) => api.sendToDevice(p.type, p.encrypted, p.messages),
  received: async () => received.splice(0),
};

async function command(input) {
  assert.equal(busy, false, "commands must be serialized");
  busy = true;
  try {
    if (input.command === "exit") {
      exiting = true;
      emit({ id: input.id, closed: true });
      process.exit(0);
    }
    const call = calls[input.command];
    assert.ok(call, `unknown command: ${input.command}`);
    let result;
    try {
      result = await call(input.params ?? {});
    } catch (error) {
      // 驱动拒绝时 matrix-widget-api 抛带 message 的 Error：这是要断言的结果，不是夹具出错。
      emit({ id: input.id, error: String(error?.message ?? error) });
      return;
    }
    emit({ id: input.id, result: result ?? null });
  } finally {
    busy = false;
  }
}

let first = true;
const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
  try {
    const input = JSON.parse(line);
    if (first) {
      first = false;
      void initialize(input).catch(fail);
    } else if (Object.hasOwn(input, "script")) {
      inject(input.script);
    } else {
      void command(input).catch(fail);
    }
  } catch (error) { fail(error); }
});
lines.on("close", () => {
  if (!exiting) fail(new Error("stdin closed without exit command"));
});
