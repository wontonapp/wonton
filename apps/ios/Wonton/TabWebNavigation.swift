import WebKit
import WontonClient

extension TabWebSession: WKNavigationDelegate {
    func receive(_ message: WKScriptMessage) {
        guard let input, let body = message.body as? String,
              let view = message.webView, view === webView else { return }
        let origin = message.frameInfo.securityOrigin
        var components = URLComponents()
        components.scheme = origin.protocol
        components.host = origin.host
        if origin.port > 0 { components.port = origin.port }
        let result = core.tabHandleMessage(
            tab: tab, isMainFrame: message.frameInfo.isMainFrame,
            origin: components.string ?? "", message: body, environment: input.environment
        )
        if result.initialized { ready = true }
        if let reply = result.replyScript {
            view.evaluateJavaScript(reply) { [weak self, weak view] _, error in
                guard let self, self.webView === view else { return }
                if let error { self.problem = error.problem; return }
                if let action = result.action { self.action?(action) }
            }
        } else if let action = result.action { self.action?(action) }
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        ready = hasContent
        navigationURL = webView.url
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        ready = true
        hasContent = true
        // 不透明时 WebKit 才在页面之外（底栏、Home 条后面）画页面自己的背景色；透明时露出的是 App 底色，
        // 内容在容器里滚、只有一屏高的页面底下就是一条白（iOS 26.5 实测）。
        webView.isOpaque = true
        recoveryURL = nil
        canGoBack = webView.canGoBack
        do {
            try core.dispatch(action: .tabLoadSucceeded(page: .bar(tab: tab)))
            try updateContext(webView, force: true)
        } catch { problem = error.problem }
        #if DEBUG
        let path = webView.url?.path ?? ""
        NSLog("Wonton tab loaded: %@ %@", String(describing: tab), path)
        #endif
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor response: WKNavigationResponse,
        decisionHandler: @escaping @MainActor (WKNavigationResponsePolicy) -> Void
    ) {
        guard response.isForMainFrame, let http = response.response as? HTTPURLResponse else {
            decisionHandler(.allow)
            return
        }
        let tunnelFailed = http.value(forHTTPHeaderField: "x-wonton-tunnel-error") != nil
        guard tunnelFailed || http.statusCode >= 500 else {
            decisionHandler(.allow)
            return
        }
        decisionHandler(.cancel)
        recoveryURL = response.response.url
        loadFailed(status: tunnelFailed ? nil : UInt16(clamping: http.statusCode))
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor navigation: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        guard navigation.targetFrame?.isMainFrame != false, let url = navigation.request.url else {
            decisionHandler(.allow)
            return
        }
        let entry = input?.page.url.flatMap(URL.init(string:))
        let domain = input?.proxy?.domain
        let authHost = domain.map { "auth.\($0)" }
        // OIDC 登录会经过核心的这段路径；系统浏览器没有隧道和设备身份，走不完。
        let coreLogin = url.host == domain && url.path.hasPrefix("/_wonton/oidc/")
        let local = (url.scheme == "http" || url.scheme == "https")
            && (url.host == entry?.host || url.host == authHost || coreLogin)
        if local {
            if navigation.targetFrame == nil {
                decisionHandler(.cancel)
                webView.load(navigation.request)
            } else {
                decisionHandler(.allow)
            }
        } else {
            decisionHandler(.cancel)
            action?(.openLink(url: url.absoluteString))
        }
    }

    func webView(
        _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
    ) {
        failed(error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        failed(error)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) { rebuild() }

    private func failed(_ error: Error) {
        let error = error as NSError
        let cancelled = error.domain == NSURLErrorDomain && error.code == NSURLErrorCancelled
        let policy = error.domain == "WebKitErrorDomain" && error.code == 102
        if !cancelled && !policy { loadFailed(status: nil) }
    }

    private func loadFailed(status: UInt16?) {
        ready = hasContent
        if recoveryURL == nil { recoveryURL = navigationURL }
        do {
            try core.dispatch(action: .tabLoadFailed(page: .bar(tab: tab), status: status))
        } catch { problem = error.problem }
    }
}
