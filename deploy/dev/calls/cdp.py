#!/usr/bin/env python3
"""对 App 里 widget 页（系统 WebView）说 CDP：列页、求值、注入先行脚本、抓 WebRTC 统计。

用法（冒烟脚本调，不直接用）：
  cdp.py pages <端口>                          列出页面，每行「id<TAB>url」
  cdp.py pick <端口>                           打印 widget 页的 id（插件页地址，不含 devtools 自身的页）
  cdp.py eval <端口> <页面 id> <表达式>         求值并打印 JSON 结果
  cdp.py addscript <端口> <页面 id> <文件>      在页面每次加载前注入脚本
  cdp.py stats <端口> <页面 id> [次数]          每 2 秒取一次 RTCPeerConnection 统计，打印 JSON 行
  cdp.py watch <端口> <脚本文件>               常驻：每个新页面加载前注入脚本

页面 id 是 pages 给的那个（WebView 里一个 widget 页一个）。

App 的 Debug 版本开着 WebView 调试（PluginWebView.kt），模拟器上是
`adb shell cat /proc/net/unix` 里带 webview_devtools_remote_<数字> 的抽象套接字；冒烟脚本用
`adb forward tcp:<端口> localabstract:<套接字>` 打通，再把 http://127.0.0.1:<端口>/json 的页面连过来。
CDP 的 WebSocket 校验 Origin，连的时候要 suppress_origin=True。
"""
import json
import sys
import time
import urllib.request

from websocket import create_connection


HOST = "127.0.0.1"

# widget 页的地址特征：插件自己的域名（`<插件>.<实例>.wonton.localhost:<端口>`），
# 插件内嵌的 livekit 帧，和元素通话自己的 `/call` 路径。devtools 自身的管理页不算。
PICK_NEEDLES = ("app-element-call", ".wonton.localhost", "livekit", "/call")


class Page:
    def __init__(self, port, page_id):
        pages = json.load(urllib.request.urlopen(f"http://{HOST}:{port}/json", timeout=10))
        match = [p for p in pages if p.get("id") == page_id]
        if not match:
            raise SystemExit(f"没有页面 {page_id}（有：{[p.get('id') for p in pages]}）")
        self.ws = create_connection(match[0]["webSocketDebuggerUrl"].replace("127.0.0.1", HOST), timeout=20, suppress_origin=True)
        self.mid = 0
        # Page 域方法（addScriptToEvaluateOnNewDocument 等）不 enable 会被静默拒绝。
        for m in ("Page.enable", "Runtime.enable"):
            r = self.send(m)
            if "error" in r:
                raise SystemExit(f"{m} 失败：{r['error']}")

    def send(self, method, params=None):
        self.mid += 1
        self.ws.send(json.dumps({"id": self.mid, "method": method, "params": params or {}}))
        while True:
            msg = json.loads(self.ws.recv())
            if msg.get("id") == self.mid:
                return msg

    def eval(self, expression, context=None):
        reply = self.send(
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True, "awaitPromise": True, **({"contextId": int(context)} if context else {})},
        )
        result = reply.get("result", {})
        if "exceptionDetails" in result:
            return {"exception": result["exceptionDetails"].get("text"), "value": None}
        return {"exception": None, "value": result.get("result", {}).get("value")}


class Browser:
    def __init__(self, port):
        version = json.load(urllib.request.urlopen(f"http://{HOST}:{port}/json/version", timeout=10))
        url = version["webSocketDebuggerUrl"].replace("127.0.0.1", HOST)
        self.ws = create_connection(url, timeout=20, suppress_origin=True)
        self.ws.settimeout(None)
        self.mid = 0
        self.pending = {}

    def send(self, method, params=None, session=None):
        self.mid += 1
        message = {"id": self.mid, "method": method, "params": params or {}}
        if session:
            message["sessionId"] = session
        self.ws.send(json.dumps(message))
        return self.mid

    def wait(self, mid):
        while mid not in self.pending:
            msg = json.loads(self.ws.recv())
            if "id" in msg:
                self.pending[msg["id"]] = msg
        return self.pending.pop(mid)


def watch(port, premain_path):
    """每个新页面加载前注入 premain：目标挂上就注入，然后放行它继续加载。"""
    source = open(premain_path, encoding="utf-8").read()
    browser = Browser(port)
    browser.wait(browser.send("Target.setAutoAttach", {"autoAttach": True, "waitForDebuggerOnStart": True, "flatten": True}))
    print("已挂上，等页面", flush=True)
    while True:
        msg = json.loads(browser.ws.recv())
        if msg.get("method") != "Target.attachedToTarget":
            continue
        params = msg["params"]
        if params.get("targetInfo", {}).get("type") != "page":
            browser.send("Runtime.runIfWaitingForDebugger", session=params["sessionId"])
            continue
        session = params["sessionId"]
        browser.wait(browser.send("Page.addScriptToEvaluateOnNewDocument", {"source": source}, session=session))
        browser.send("Runtime.runIfWaitingForDebugger", session=session)
        print(f"已注入 {params['targetInfo'].get('targetId')}", flush=True)


def main(argv):
    if len(argv) < 3:
        raise SystemExit(__doc__)
    global HOST
    args = argv[1:]
    port = None
    while args and args[0] in ("--host", "--port"):
        key = args.pop(0)
        value = args.pop(0)
        if key == "--host":
            HOST = value
        else:
            port = value
    command = args.pop(0)
    if port is None:
        port = args.pop(0)
    if command == "watch":
        watch(port, args[0])
        return 0
    if command == "pages":
        pages = json.load(urllib.request.urlopen(f"http://{HOST}:{port}/json", timeout=10))
        for page in pages:
            print(f"{page['id']}\t{page.get('url', '')}")
        return 0
    if command == "pick":
        pages = json.load(urllib.request.urlopen(f"http://{HOST}:{port}/json", timeout=10))
        # 优先 app-element-call：.wonton.localhost 会命中任何 App 页，顺序敏感会选错。
        hit = [p for p in pages if "app-element-call" in p.get("url", "")]
        if not hit:
            hit = [p for p in pages if any(n in p.get("url", "") for n in PICK_NEEDLES)]
        if not hit:
            raise SystemExit(f"没有 widget 页（有：{[(p['id'], p.get('url', '')) for p in pages]}）")
        print(hit[-1]["id"])
        return 0
    if not args:
        raise SystemExit(__doc__)
    page = Page(port, args.pop(0))
    if command == "eval":
        print(json.dumps(page.eval(args[0]), ensure_ascii=False))
    elif command == "click":
        page.send("Input.dispatchMouseEvent", {"type": "mousePressed", "x": float(args[0]), "y": float(args[1]),
                                               "button": "left", "clickCount": 1})
        page.send("Input.dispatchMouseEvent", {"type": "mouseReleased", "x": float(args[0]), "y": float(args[1]),
                                               "button": "left", "clickCount": 1})
        print("clicked")
    elif command == "addscript":
        source = open(args[0], encoding="utf-8").read()
        page.send("Page.addScriptToEvaluateOnNewDocument", {"source": source})
        print("已注入，重新加载页面后生效")
    elif command == "evalscript":
        source = open(args[0], encoding="utf-8").read()
        value = page.eval(source, args[1] if len(args) > 1 else None)
        print(json.dumps(value, ensure_ascii=False))
    elif command == "contexts":
        page.mid += 1
        page.ws.send(json.dumps({"id": page.mid, "method": "Runtime.enable"}))
        page.ws.settimeout(3)
        out = 0
        try:
            while out < 50:
                msg = json.loads(page.ws.recv())
                if msg.get("method") == "Runtime.executionContextCreated":
                    ctx = msg["params"]["context"]
                    print(f"{ctx['id']}\t{ctx.get('origin', '')}\t{ctx.get('name', '')}")
                    out += 1
        except Exception:
            pass
    elif command == "session":
        # 一条连接内装钩子再采样：页 target 在组流程里会换代，分开的注入与采样常落在不同页。
        # 装好后先等 EC 的 getStats 循环把活连接喂进 __pcs（≤15s），再连续采样。
        src = open(args[0], encoding="utf-8").read()
        samples = int(args[1]) if len(args) > 1 else 10
        page.eval(src)
        t = page.eval("typeof window.__pcs")["value"]
        print(f"__pcs {t}", file=sys.stderr)
        if t == "object":
            for _ in range(5):
                n = page.eval("(window.__pcs||[]).length")["value"] or 0
                if int(n) > 0:
                    break
                time.sleep(3)
        for _ in range(samples):
            time.sleep(2)
            print(json.dumps(page.eval(JS_STATS)["value"], ensure_ascii=False))
    elif command == "reload":
        page.send("Page.reload")
    elif command == "stats":
        context = args[0] if args and args[0] != "-" else None
        samples = int(args[1]) if len(args) > 1 else 10
        for _ in range(samples):
            time.sleep(2)
            value = page.eval(JS_STATS, context)["value"]
            print(json.dumps(value, ensure_ascii=False))
    else:
        raise SystemExit(__doc__)
    return 0


# 记在 window.__pcs 的连接由 premain.js 包 RTCPeerConnection 时收进来。取选中候选对、收发码率、丢包和帧数。
JS_STATS = r"""
(async () => {
  const out = {at: Date.now(), pcs: []};
  for (const pc of (window.__pcs || [])) {
    const row = {state: pc.connectionState, candidates: [], inbound: {}, outbound: {}};
    const stats = await pc.getStats();
    for (const report of stats.values()) {
      if (report.type === 'candidate-pair' && (report.nominated || report.selected)) {
        row.selected = {local: report.localCandidateId, remote: report.remoteCandidateId,
                        rtt: report.currentRoundTripTime, bytesSent: report.bytesSent,
                        bytesReceived: report.bytesReceived};
      }
      if (report.type === 'local-candidate' || report.type === 'remote-candidate') {
        row.candidates.push({id: report.id, side: report.type, type: report.candidateType, protocol: report.protocol,
                             address: report.address, port: report.port});
      }
      if (report.type === 'inbound-rtp' && report.kind === 'video') {
        row.inbound.video = {bytes: report.bytesReceived, frames: report.framesDecoded,
                             lost: report.packetsLost, width: report.frameWidth, height: report.frameHeight};
      }
      if (report.type === 'outbound-rtp' && report.kind === 'video') {
        row.outbound.video = {bytes: report.bytesSent, frames: report.framesEncoded,
                              width: report.frameWidth, height: report.frameHeight};
      }
      if (report.type === 'inbound-rtp' && report.kind === 'audio') {
        row.inbound.audio = {bytes: report.bytesReceived, lost: report.packetsLost};
      }
      if (report.type === 'outbound-rtp' && report.kind === 'audio') {
        row.outbound.audio = {bytes: report.bytesSent};
      }
    }
    out.pcs.push(row);
  }
  return out;
})()
"""

if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
