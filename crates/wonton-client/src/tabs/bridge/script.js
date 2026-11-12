(state, install, widget) => {
  if (window.parent !== window) return;

  const apply = () => {
    const root = document.documentElement;
    if (!root) return false;
    for (const [name, value] of Object.entries(state.variables)) {
      root.style.setProperty(name, value);
    }
    root.style.setProperty("color-scheme", state.context.theme);
    root.style.setProperty("background-color", state.variables["--color-background-primary"]);
    return true;
  };
  if (!apply()) {
    // document-start 可能早于解析器创建 <html>。
    const observer = new MutationObserver(() => {
      if (apply()) observer.disconnect();
    });
    observer.observe(document, { childList: true });
  }

  if (!install) {
    window.dispatchEvent(new MessageEvent("message", {
      data: {
        jsonrpc: "2.0",
        method: "ui/notifications/host-context-changed",
        params: state.context,
      },
      source: window,
    }));
    return;
  }

  // widget API（MSC2762）里页面发出的两种：fromWidget 请求、对 toWidget 请求的应答。只在 widget 页接。
  const fromWidget = (data) => widget === true
    && ((data?.api === "fromWidget" && data.response === undefined)
      || (data?.api === "toWidget" && data.response !== undefined));
  window.addEventListener("message", (event) => {
    if (event.source !== window) return;
    if (event.data?.jsonrpc !== "2.0" && !fromWidget(event.data)) return;
    // 宿主注入的合成 MessageEvent 使用空 origin。
    // 真实 postMessage 事件始终携带页面来源（或 "null"）。
    if (event.origin === "") return;
    event.stopImmediatePropagation();
    const message = JSON.stringify(event.data);
    const ios = window.webkit?.messageHandlers?.wontonBridge;
    if (ios) {
      ios.postMessage(message);
    } else if (window.wontonBridge?.postMessage) {
      window.wontonBridge.postMessage(message);
    } else {
      throw new Error("Wonton native message channel is unavailable");
    }
  }, true);

}
