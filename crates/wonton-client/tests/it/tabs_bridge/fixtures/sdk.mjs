import assert from "node:assert/strict";
import { createInterface } from "node:readline";
import { runInThisContext } from "node:vm";
import { format } from "node:util";
import { App } from "./generated/ext-apps-2.0.4.mjs";

const emit = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
for (const method of ["log", "debug", "info", "warn", "error"]) {
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
class WindowCustomEvent extends Event {
  constructor(type, { detail = null } = {}) {
    super(type);
    this.detail = detail;
  }
}
class FakeWindow extends EventTarget {
  postMessage(data, targetOrigin = "*") {
    assert.ok(targetOrigin === "*" || targetOrigin === this.location.origin);
    const copy = structuredClone(data);
    setImmediate(() => this.dispatchEvent(new WindowMessageEvent("message", {
      data: copy, origin: this.location.origin, source: this,
    })));
  }
}
class Style {
  setProperty(name, value) { this[name] = String(value); }
  getPropertyValue(name) { return this[name] ?? ""; }
  removeProperty(name) {
    const previous = this.getPropertyValue(name);
    delete this[name];
    return previous;
  }
}
const mutations = new Set();
class RootObserver {
  constructor(callback) { this.callback = callback; }
  observe(target, options) {
    assert.equal(target, document);
    assert.equal(options.childList, true);
    mutations.add(this);
  }
  disconnect() { mutations.delete(this); }
}
const events = [];
const contextChanges = [];
let app;
let connected = false;
let busy = false;
let exiting = false;
const turn = () => new Promise((resolve) => setImmediate(resolve));

function installWindow(platform, url) {
  assert.ok(platform === "ios" || platform === "android");
  const window = new FakeWindow();
  const document = new EventTarget();
  document.documentElement = null;
  document.readyState = "loading";
  window.parent = window;
  window.self = window;
  window.window = window;
  window.document = document;
  window.location = new URL(url);
  window.MessageEvent = WindowMessageEvent;
  window.CustomEvent = WindowCustomEvent;
  window.MutationObserver = RootObserver;
  const channel = { postMessage(message) {
    assert.equal(typeof message, "string");
    JSON.parse(message);
    emit({ message, origin: window.location.origin });
  } };
  if (platform === "ios") window.webkit = { messageHandlers: { wontonBridge: channel } };
  else window.wontonBridge = channel;
  Object.assign(globalThis, {
    window, document, self: window, parent: window, location: window.location,
    MessageEvent: WindowMessageEvent, CustomEvent: WindowCustomEvent,
    MutationObserver: RootObserver,
  });
  for (const type of ["wonton:tab-reselected", "wonton:connection"]) {
    window.addEventListener(type, (event) => events.push({ type, detail: event.detail ?? null }));
  }
}
function inject(script) {
  assert.equal(typeof script, "string");
  runInThisContext(script, { filename: "native-injection.js", timeout: 1000 });
  assert.equal(Object.hasOwn(window, "wonton"), false);
}
async function initialize(input) {
  installWindow(input.platform, input.url);
  inject(input.script);
  // The injection runs at document-start, before the parser creates <html>.
  await turn();
  document.documentElement = { style: new Style() };
  for (const observer of [...mutations]) observer.callback([{ type: "childList", target: document }], observer);
  await turn();
  document.readyState = "interactive";
  document.dispatchEvent(new Event("DOMContentLoaded"));
  app = new App({ name: "wonton-tabs-fixture", version: "1.0.0" }, {}, { autoResize: false });
  app.onerror = fail;
  app.onhostcontextchanged = (context) => contextChanges.push(context);
  await app.connect();
  // postMessage is asynchronous: flush the initialized notification before ready.
  await turn();
  connected = true;
  emit({ ready: true, context: app.getHostContext(), capabilities: app.getHostCapabilities() });
}
async function command(input) {
  assert.equal(connected, true, "command before SDK ready");
  assert.equal(busy, false, "commands must be serialized; reply injections may interleave");
  assert.ok(Object.hasOwn(input, "id"));
  busy = true;
  try {
    await turn();
    if (input.command === "inspect") {
      emit({ id: input.id, context: app.getHostContext(), capabilities: app.getHostCapabilities(),
        css: { ...document.documentElement.style }, events, contextChanges });
    } else if (input.command === "exit") {
      exiting = true;
      await app.close();
      emit({ id: input.id, closed: true });
      process.exit(0);
    } else {
      await callSdk(input);
    }
  } finally {
    busy = false;
  }
}
async function callSdk(input) {
  const methods = new Set(["openLink", "sendMessage", "requestDisplayMode", "sendSizeChanged"]);
  assert.ok(methods.has(input.command), `unknown command: ${input.command}`);
  let result;
  try {
    result = await app[input.command](input.params);
  } catch (error) {
    if (input.expectError !== true) throw error;
    assert.equal(typeof error.code, "number", "expected a JSON-RPC error, not a harness failure");
    emit({ id: input.id, error: { code: error.code, message: error.message } });
    return;
  }
  assert.notEqual(input.expectError, true, "SDK call unexpectedly succeeded");
  await turn();
  emit({ id: input.id, result: result ?? null });
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
