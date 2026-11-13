import WebKit
import WontonClient

extension TabWebSession: WKNavigationDelegate {
    func receive(_ message: WKScriptMessage) {
        guard let input, let view = message.webView, view === webView else { return }
        if message.name == "wontonMedia" {
            receiveMedia(message, view: view)
            return
        }
        guard let body = message.body as? String else { return }
        let origin = Self.originString(message.frameInfo.securityOrigin)
        let result = core.tabHandleMessage(
            page: placed, isMainFrame: message.frameInfo.isMainFrame,
            origin: origin, message: body, environment: input.environment
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

    /// 网页报来的媒体状态：开始或停止放就上报核心，后台照常探测、自动重连（ADR-0033）。
    /// 通道只开给页面自己的源，页面里嵌的第三方 iframe 报的不算（同 Android 把通道限制到插件的源）。
    private func receiveMedia(_ message: WKScriptMessage, view: WKWebView) {
        guard message.frameInfo.isMainFrame, let body = message.body as? String,
              Self.sameOrigin(message.frameInfo.securityOrigin, of: view) else { return }
        if let state = core.pageMediaMessage(message: body) {
            setMediaPlaying(state.playback == .playing)
        }
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        ready = hasContent
        navigationURL = webView.url
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        ready = true
        hasContent = true
        // 不透明时 WebKit 才在页面之外（回弹露出的那块）画页面自己的背景色；透明时露出的是 App 底色，
        // 内容在容器里滚、只有一屏高的页面底下就是一条白（iOS 26.5 实测）。
        webView.isOpaque = true
        recoveryURL = nil
        canGoBack = webView.canGoBack
        do {
            try core.dispatch(action: .tabLoadSucceeded(page: placed))
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
        // 类别由代理给（x-wonton-error）。网关的响应也用这个头名，只认代理自己的取值：
        // tunnel-unavailable 是没拿到响应，upstream-unavailable 拿到了响应（服务器在，上游没就绪），
        // 照实交状态码给核心判断。
        let category = http.value(forHTTPHeaderField: "x-wonton-error").map { $0.lowercased() }
        let proxyCategories: Set = ["tunnel-unavailable", "upstream-unavailable", "unauthorized", "protocol"]
        let proxyCategory = category.map { proxyCategories.contains($0) } == true
        let tunnelFailed = proxyCategory
            || http.value(forHTTPHeaderField: "x-wonton-tunnel-error") != nil
        let sharedRefusal: Bool
        if case .sharedApp = tab {
            sharedRefusal = ["share_identity", "share_required", "share_revoked", "share_expired"]
                .contains(http.value(forHTTPHeaderField: "x-wonton-error") ?? "")
        } else { sharedRefusal = false }
        guard tunnelFailed || sharedRefusal || http.statusCode >= 500 else {
            decisionHandler(.allow)
            return
        }
        decisionHandler(.cancel)
        recoveryURL = response.response.url
        let status: UInt16?
        switch category {
        case "upstream-unavailable": status = UInt16(clamping: http.statusCode)
        case "tunnel-unavailable", "unauthorized", "protocol": status = nil
        default: status = tunnelFailed ? nil : UInt16(clamping: http.statusCode)
        }
        loadFailed(status: status)
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor navigation: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        guard navigation.targetFrame?.isMainFrame != false, let url = navigation.request.url else {
            decisionHandler(.allow)
            return
        }
        // 公网网页在同一个 WebView 里随便走 http/https，别的 scheme（mailto 等）交给系统。
        if case .url = placed.tab {
            let scheme = url.scheme?.lowercased()
            if scheme == "http" || scheme == "https" {
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
            try core.dispatch(action: .tabLoadFailed(page: placed, status: status))
        } catch { problem = error.problem }
    }
}
