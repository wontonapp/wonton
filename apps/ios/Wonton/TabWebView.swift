import Network
import Observation
import SwiftUI
import WebKit
import WontonClient

@MainActor
final class TabWebViewStore {
    private var sessions: [PlacedTab: TabWebSession] = [:]
    private var sharedStores: [String: WKWebsiteDataStore] = [:]
    private var accountId: String?
    /// 释放过 WebView 的计数：页面视图据此判断手里的会话还能不能用、要不要重新要一份。
    private(set) var generation = 0

    func session(for placed: PlacedTab, core: ClientCore) -> TabWebSession {
        let currentAccount = core.servers().servers.first(where: { $0.current })?.id
        if accountId != currentAccount {
            for session in sessions.values { session.dispose() }
            sessions.removeAll()
            sharedStores.removeAll()
            accountId = currentAccount
        }
        if let session = sessions[placed] { return session }
        let store: WKWebsiteDataStore
        if case let .sharedApp(ownerUserId, _) = placed.tab {
            if let retained = sharedStores[ownerUserId] { store = retained } else {
                store = .nonPersistent()
                sharedStores[ownerUserId] = store
            }
        } else { store = .default() }
        let session = TabWebSession(placed: placed, core: core, dataStore: store)
        sessions[placed] = session
        return session
    }

    /// 整页和面板里的同一个页面各一份（ADR-0052）：不在这一份里的释放掉。
    func retain(_ placed: Set<PlacedTab>) {
        for key in Array(sessions.keys) where !placed.contains(key) {
            sessions.removeValue(forKey: key)?.dispose()
        }
        generation += 1
    }

    func releaseAll() {
        for session in sessions.values { session.dispose() }
        sessions.removeAll()
        generation += 1
    }
}

extension PlacedTab {
    var tab: TabRef {
        switch self {
        case let .page(tab), let .panel(_, tab): tab
        }
    }
}

struct TabWebInputs: Equatable {
    let page: TabPage
    let proxy: ProxySettings?
    let connected: Bool
    let appearance: AppearanceViewModel
    let environment: TabEnvironment
    let textScale: CGFloat
}

/// WKWebView、历史和加载进度跟着标签页存活，不跟着 SwiftUI 的挂载次数存活。
@MainActor
@Observable
final class TabWebSession: NSObject, WKUIDelegate {
    let placed: PlacedTab
    let core: ClientCore
    var tab: TabRef { placed.tab }
    private(set) var webView: WKWebView?
    var ready = false
    var canGoBack = false
    var problem: Problem?
    @ObservationIgnored var input: TabWebInputs?
    @ObservationIgnored var action: ((TabBridgeAction) -> Void)?
    @ObservationIgnored var loadedReload: UInt64?
    @ObservationIgnored var loadedEntry: String?
    @ObservationIgnored var loadedProxy: ProxySettings?
    @ObservationIgnored var recoveryURL: URL?
    @ObservationIgnored var navigationURL: URL?
    @ObservationIgnored var hasContent = false
    @ObservationIgnored private var configuredProxy: ProxySettings?
    @ObservationIgnored var lastContext: String?
    @ObservationIgnored var lastConnection: String?
    @ObservationIgnored var documentScript: String?
    @ObservationIgnored private var lastTextScale: CGFloat?
    @ObservationIgnored private var backObservation: NSKeyValueObservation?
    @ObservationIgnored private let dataStore: WKWebsiteDataStore
    /// 这份会话的网页在放媒体：开始或停止时经 [`PageMediaActive`] 上报核心。
    @ObservationIgnored private var mediaPlaying = false

    init(placed: PlacedTab, core: ClientCore, dataStore: WKWebsiteDataStore) {
        self.placed = placed
        self.core = core
        self.dataStore = dataStore
    }

    func update(_ input: TabWebInputs, action: @escaping (TabBridgeAction) -> Void) {
        self.input = input
        self.action = action
        if case .sharedApp = tab, !input.connected {
            webView?.stopLoading()
            ready = false
            return
        }
        do {
            if let webView {
                try configure(webView)
                try updateContext(webView)
            }
            guard input.connected, input.page.state == .ready,
                  let entry = input.page.url, let url = URL(string: entry) else { return }
            // 插件页要等家里的代理；公网网页直接连，不经隧道（ADR-0052），没有代理也加载。
            let proxy = input.proxy
            guard proxy != nil || !bridged else { return }
            if webView == nil { try create() }
            guard let webView else { return }
            if loadedReload != input.page.reload || loadedEntry != entry || loadedProxy != proxy {
                let destination = loadedEntry == entry ? recoveryURL ?? webView.url ?? url : url
                ready = false
                problem = nil
                loadedReload = input.page.reload
                loadedEntry = entry
                loadedProxy = proxy
                recoveryURL = nil
                webView.load(URLRequest(url: destination))
            }
        } catch {
            problem = error.problem
        }
    }

    private func create() throws {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = Self.dataStore(for: placed)
        // 公网网页没有桥，也不注入变量和脚本（ADR-0052）。
        if bridged {
            let controller = configuration.userContentController
            controller.add(WeakTabMessageHandler(self), name: "wontonBridge")
            controller.add(WeakTabMessageHandler(self), name: "wontonMedia")
            // 插件页换曲时在新元素上自动播放，等不到用户手势；页面本来就是主人打开的（同 Android）。
            configuration.mediaTypesRequiringUserActionForPlayback = []
        }
        // 和 Safari 一样，页面写了 user-scalable=no 或 maximum-scale 也能双指缩放；
        // 配合 focusZoomScript 的 maximum-scale=1，只拦住聚焦输入框时的自动放大。
        configuration.ignoresViewportScaleLimits = true
        let view = WKWebView(frame: .zero, configuration: configuration)
        // 加载出来之前透明，露出 App 底色，不闪白；加载完改成不透明（见 didFinish）。
        view.isOpaque = false
        view.scrollView.contentInsetAdjustmentBehavior = .never
        // 网页自己的前进后退手势也是左边缘右滑，会抢系统的返回；关掉，网页历史走胶囊菜单的后退。
        view.allowsBackForwardNavigationGestures = false
        view.navigationDelegate = self
        if bridged {
            view.uiDelegate = self
            let captures = responds(to: #selector(WKUIDelegate.webView(_:requestMediaCapturePermissionFor:initiatedByFrame:type:decisionHandler:)))
            NSLog("[media] create bridged uiDelegate=session responds=%d", captures ? 1 : 0)
        } else {
            NSLog("[media] create unbridged (public page), no uiDelegate")
        }
        #if DEBUG
        view.isInspectable = true
        #endif
        try configure(view)
        webView = view
        backObservation = view.observe(\.canGoBack, options: [.initial, .new]) { [weak self] _, _ in
            Task { @MainActor in self?.canGoBack = self?.webView?.canGoBack ?? false }
        }
    }

    private func configure(_ view: WKWebView) throws {
        guard let input else { return }
        // 公网网页直接连，不经家里的隧道代理（ADR-0052）。
        if bridged, let proxy = input.proxy, configuredProxy != proxy || view !== webView {
            guard let port = NWEndpoint.Port(rawValue: proxy.port) else {
                throw CoreError.InvalidInput(reason: String(localized: .Tab0.tabProxyPortInvalid))
            }
            let endpoint = NWEndpoint.hostPort(host: NWEndpoint.Host(proxy.host), port: port)
            var configuration = ProxyConfiguration(httpCONNECTProxy: endpoint)
            configuration.applyCredential(username: proxy.username, password: proxy.password)
            configuration.matchDomains = [proxy.domain]
            view.configuration.websiteDataStore.proxyConfigurations = [configuration]
            configuredProxy = proxy
        }
        let colors = input.environment.dark ? input.appearance.dark : input.appearance.light
        let background = UIColor(Color(colors.canvas))
        view.backgroundColor = background
        view.pageZoom = 1
        guard bridged else { return }
        let script = try core.tabDocumentScript(page: placed, environment: input.environment)
            + "\n" + textSizeScript(input.textScale)
        if script != documentScript || view !== webView {
            let controller = view.configuration.userContentController
            controller.removeAllUserScripts()
            controller.addUserScript(WKUserScript(
                source: script, injectionTime: .atDocumentStart, forMainFrameOnly: true
            ))
            // 核心的媒体脚本经 window.wontonMedia 发状态（同 Android 的 WebMessageListener 名字）；
            // iOS 的消息处理挂在 window.webkit.messageHandlers 上，先垫一段把两边对上。
            controller.addUserScript(WKUserScript(
                source: Self.mediaChannelScript + "\n" + core.pageMediaScript(),
                injectionTime: .atDocumentStart, forMainFrameOnly: true
            ))
            controller.addUserScript(WKUserScript(
                source: Self.focusZoomScript, injectionTime: .atDocumentEnd, forMainFrameOnly: true
            ))
            documentScript = script
        }
    }

    func updateContext(_ view: WKWebView, force: Bool = false) throws {
        guard bridged, let input, view.url != nil else { return }
        let textChanged = input.textScale != lastTextScale
        if force || textChanged {
            evaluate(textSizeScript(input.textScale), in: view)
            lastTextScale = input.textScale
        }
        let context = try core.tabContextScript(page: placed, environment: input.environment)
        if force || textChanged || context != lastContext {
            evaluate(context, in: view)
            lastContext = context
        }
        let connection = core.tabConnectionScript(tab: tab)
        if force || connection != lastConnection {
            evaluate(connection, in: view)
            lastConnection = connection
        }
    }

    func evaluate(_ script: String, in view: WKWebView) {
        view.evaluateJavaScript(script) { [weak self, weak view] _, error in
            guard let self, self.webView === view, let error else { return }
            self.problem = error.problem
        }
    }

    /// 公网网页没有桥：不注入脚本和变量，不装消息处理。
    private var bridged: Bool {
        if case .url = placed.tab { return false }
        return true
    }

    /// 公网网页不和插件页共用 Cookie 和存储（ADR-0052）：公网页面共用一个单独的存储。
    private static let publicPanelStore = UUID(uuidString: "6C3B0E7A-6C61-4A0E-9E2B-5C5A0B7C8F61")!

    private static func dataStore(for placed: PlacedTab) -> WKWebsiteDataStore {
        guard case .url = placed.tab else { return .default() }
        return WKWebsiteDataStore(forIdentifier: publicPanelStore)
    }

    /// 核心的媒体脚本经 window.wontonMedia 发状态（同 Android 的 WebMessageListener 名字）；
    /// iOS 的消息处理挂在 window.webkit.messageHandlers 上，先垫一段把两边对上。
    private static let mediaChannelScript = """
    window.wontonMedia = {
      postMessage: (message) => window.webkit.messageHandlers.wontonMedia.postMessage(message),
    };
    """

    /// WebKit 在输入框字号小于 16px 时聚焦会放大页面，失焦不缩回（#442）。聚焦放大的上限
    /// 取 viewport 的 maximum-scale，不受 ignoresViewportScaleLimits 影响，所以补上
    /// maximum-scale=1 只拦它。页面自己写了 maximum-scale 的照它的；没有 viewport 的
    /// 是桌面宽度的页面，聚焦放大对它有用，不动。
    private static let focusZoomScript = """
        (() => {
          const meta = document.querySelector('meta[name="viewport"]');
          if (!meta || /maximum-scale/i.test(meta.content)) return;
          meta.content = meta.content + ', maximum-scale=1';
        })();
        """

    private func textSizeScript(_ scale: CGFloat) -> String {
        """
        (() => {
          const apply = () => {
            if (!document.documentElement) return false;
            document.documentElement.style.setProperty(
              '-webkit-text-size-adjust', '\(Double(scale * 100))%', 'important'
            );
            return true;
          };
          if (!apply()) {
            const observer = new MutationObserver(() => {
              if (apply()) observer.disconnect();
            });
            observer.observe(document, { childList: true });
          }
        })();
        """
    }

    /// 面板拉开、翻到这一页：经桥告诉页面它被看到了。
    func seen() {
        guard let webView else { return }
        evaluate(core.tabReselectedScript(), in: webView)
    }

    /// 网页开始或停止放媒体：上报核心，App 退到后台也照常探测、自动重连（ADR-0033）。
    /// 整页和面板里的同一页各一份会话，都停了才解除，换曲的一瞬由核心的宽限兜住。
    func setMediaPlaying(_ playing: Bool) {
        guard mediaPlaying != playing else { return }
        mediaPlaying = playing
        PageMediaActive.shared.set(self, playing: playing, core: core)
    }

    /// 原生回调里的来源拼成 URL 字符串（桥的来源同款），核心按它和页面自己的源核对。
    static func originString(_ origin: WKSecurityOrigin) -> String {
        var components = URLComponents()
        components.scheme = origin.protocol
        components.host = origin.host
        if origin.port > 0 { components.port = origin.port }
        return components.string ?? ""
    }

    /// 同一个源：scheme、主机和有效端口（缺省按方案的默认口）。桥的来源判定是同一条规矩，
    /// 媒体通道核心不核对，壳在收消息时照它核对。
    static func sameOrigin(_ origin: WKSecurityOrigin, of view: WKWebView) -> Bool {
        guard let url = view.url else { return false }
        let scheme = url.scheme?.lowercased()
        guard origin.protocol.lowercased() == scheme,
              origin.host.lowercased() == url.host?.lowercased() else { return false }
        let port = origin.port > 0 ? origin.port : defaultPort(scheme)
        return port == (url.port ?? defaultPort(scheme))
    }

    private static func defaultPort(_ scheme: String?) -> Int {
        switch scheme {
        case "http": 80
        case "https": 443
        default: 0
        }
    }

    func retry() {
        problem = nil
        do {
            if case let .sharedApp(ownerUserId, _) = tab {
                try core.dispatch(action: .refreshSharedApps(ownerUserId: ownerUserId))
            } else {
                try core.dispatch(action: .refreshPlugins)
            }
            try core.dispatch(action: .reloadTab(page: placed))
        } catch { problem = error.problem }
    }

    func rebuild() {
        guard let input, let action else { return }
        recoveryURL = webView?.url
        dispose()
        ready = false
        hasContent = false
        configuredProxy = nil
        canGoBack = false
        loadedReload = nil
        loadedProxy = nil
        documentScript = nil
        lastContext = nil
        lastConnection = nil
        lastTextScale = nil
        update(input, action: action)
    }

    /// Remove a previously loaded page when exact Bot metadata or pre-open consent no longer permits it.
    func withhold() {
        dispose()
        input = nil
        ready = false
        hasContent = false
        canGoBack = false
        loadedReload = nil
        loadedEntry = nil
        loadedProxy = nil
        recoveryURL = nil
        navigationURL = nil
    }

    func dispose() {
        setMediaPlaying(false)
        backObservation = nil
        webView?.stopLoading()
        webView?.navigationDelegate = nil
        webView?.uiDelegate = nil
        if bridged {
            let controller = webView?.configuration.userContentController
            controller?.removeScriptMessageHandler(forName: "wontonBridge")
            controller?.removeScriptMessageHandler(forName: "wontonMedia")
        }
        webView = nil
        action = nil
    }

    /// 网页要摄像头/麦克风（getUserMedia）：先问核心（按插件记决定），按回答放行或拒绝；
    /// 放行还要系统运行时权限。其他 UI 代理方法走 WebKit 的默认。
    /// 实现放在类主体；签名按 iOS 26 SDK 的 requestMediaCapturePermissionForOrigin:
    /// （旧请求方法的 WKFrameInfo 版本已从协议里删掉，签名对不上时可选协议不报错，
    /// WebKit 直接走自己的兜底权限弹窗，绕过核心的逐插件确认）。
    func webView(
        _ webView: WKWebView,
        requestMediaCapturePermissionForOrigin origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo,
        type: WKMediaCaptureType,
        decisionHandler: @escaping @MainActor (WKPermissionDecision) -> Void
    ) {
        NSLog("[media] capture entry type=%d", type == .camera ? 0 : type == .microphone ? 1 : 2)
        let camera = type == .camera || type == .cameraAndMicrophone
        let microphone = type == .microphone || type == .cameraAndMicrophone
        // 源从 WebView 的原生回调取，核心按页面自己的源核对；页面里嵌的其他源核心直接拒。
        // 核心问不出来（页面关了、源不符、查记录失败）就是拒：如实拒网页并留日志，不静默。
        let decision: TabMediaPermission?
        do {
            decision = try core.tabMediaPermissionRequest(
                page: placed, origin: Self.originString(origin),
                camera: camera, microphone: microphone
            )
            let what = if decision == .allow {
                "allow"
            } else if decision == .prompt {
                "prompt"
            } else {
                "deny"
            }
            NSLog("[media] request ok %@ microphone=%d", what, microphone ? 1 : 0)
        } catch {
            decision = nil
            NSLog("[media] request failed origin=%@ error=%@",
                  Self.originString(origin), String(describing: error))
        }
        let answer = { (allow: Bool) in decisionHandler(allow ? .grant : .deny) }
        switch decision {
        case .allow:
            TabMediaRequests.shared.systemStep(for: type, answer: answer)
        case .prompt:
            TabMediaRequests.shared.asked(for: type, answer: answer)
        default:
            answer(false)
        }
    }
}

struct TabWebView: UIViewRepresentable {
    let webView: WKWebView
    /// 面板挡住的上下两条（ADR-0052）：页面照样画在后面，但固定定位的元素和滚动的终点让开。整页是 0（ADR-0095）。
    let obscured: UIEdgeInsets

    func makeUIView(context: Context) -> WKWebView {
        updateUIView(webView, context: context)
        return webView
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {
        guard #available(iOS 26, *), uiView.obscuredContentInsets != obscured else { return }
        uiView.obscuredContentInsets = obscured
    }
}

@MainActor
private final class WeakTabMessageHandler: NSObject, WKScriptMessageHandler {
    weak var session: TabWebSession?

    init(_ session: TabWebSession) { self.session = session }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        session?.receive(message)
    }
}
