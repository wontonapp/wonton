(state, install) => {
  if (window.parent !== window) return;

  const apply = () => {
    const root = document.documentElement;
    if (!root) return false;
    for (const [name, value] of Object.entries(state.variables)) {
      root.style.setProperty(name, value);
    }
    root.style.setProperty("color-scheme", state.context.theme);
    // 不设背景色：写在 <html> 上会盖掉页面自己的背景，也挡住 body 的背景铺到整个画布，铺满时状态栏下面露出的就不是页面的颜色（ADR-0095）。
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

  window.addEventListener("message", (event) => {
    if (event.source !== window || event.data?.jsonrpc !== "2.0") return;
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
