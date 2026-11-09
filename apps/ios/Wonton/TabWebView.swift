import Network
import Observation
import SwiftUI
import WebKit
import WontonClient

@MainActor
final class TabWebViewStore {
    private var sessions: [TabRef: TabWebSession] = [:]

    func session(for tab: TabRef, core: ClientCore) -> TabWebSession {
        if let session = sessions[tab] { return session }
        let session = TabWebSession(tab: tab, core: core)
        sessions[tab] = session
        return session
    }

    func retain(_ tabs: Set<TabRef>) {
        for tab in Array(sessions.keys) where !tabs.contains(tab) {
            sessions.removeValue(forKey: tab)?.dispose()
        }
    }

    func reselect(_ tab: TabRef) { sessions[tab]?.reselect() }
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
final class TabWebSession: NSObject {
    let tab: TabRef
    let core: ClientCore
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

    init(tab: TabRef, core: ClientCore) {
        self.tab = tab
        self.core = core
    }

    func update(_ input: TabWebInputs, action: @escaping (TabBridgeAction) -> Void) {
        self.input = input
        self.action = action
        do {
            if let webView {
                try configure(webView)
                try updateContext(webView)
            }
            guard input.connected, input.page.state == .ready,
                  let proxy = input.proxy, let entry = input.page.url,
                  let url = URL(string: entry) else { return }
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
        configuration.websiteDataStore = .default()
        configuration.userContentController.add(WeakTabMessageHandler(self), name: "wontonBridge")
        // 和 Safari 一样，页面写了 user-scalable=no 或 maximum-scale 也能双指缩放；
        // 配合 focusZoomScript 的 maximum-scale=1，只拦住聚焦输入框时的自动放大。
        configuration.ignoresViewportScaleLimits = true
        let view = WKWebView(frame: .zero, configuration: configuration)
        // 加载出来之前透明，露出 App 底色，不闪白；加载完改成不透明（见 didFinish）。
        view.isOpaque = false
        view.scrollView.contentInsetAdjustmentBehavior = .never
        view.allowsBackForwardNavigationGestures = true
        view.navigationDelegate = self
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
        if let proxy = input.proxy, configuredProxy != proxy || view !== webView {
            guard let port = NWEndpoint.Port(rawValue: proxy.port) else {
                throw CoreError.InvalidInput(reason: String(localized: .tabProxyPortInvalid))
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
        let script = try core.tabDocumentScript(tab: tab, environment: input.environment)
            + "\n" + textSizeScript(input.textScale)
        if script != documentScript || view !== webView {
            let controller = view.configuration.userContentController
            controller.removeAllUserScripts()
            controller.addUserScript(WKUserScript(
                source: script, injectionTime: .atDocumentStart, forMainFrameOnly: true
            ))
            controller.addUserScript(WKUserScript(
                source: Self.focusZoomScript, injectionTime: .atDocumentEnd, forMainFrameOnly: true
            ))
            documentScript = script
        }
    }

    func updateContext(_ view: WKWebView, force: Bool = false) throws {
        guard let input, view.url != nil else { return }
        let textChanged = input.textScale != lastTextScale
        if force || textChanged {
            evaluate(textSizeScript(input.textScale), in: view)
            lastTextScale = input.textScale
        }
        let context = try core.tabContextScript(tab: tab, environment: input.environment)
        if force || textChanged || context != lastContext {
            evaluate(context, in: view)
            lastContext = context
        }
        let connection = core.tabConnectionScript()
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

    func reselect() {
        guard let webView else { return }
        webView.scrollView.setContentOffset(.zero, animated: !UIAccessibility.isReduceMotionEnabled)
        evaluate(core.tabReselectedScript(), in: webView)
    }

    func retry() {
        problem = nil
        do {
            try core.dispatch(action: .refreshPlugins)
            try core.dispatch(action: .reloadTab(page: .bar(tab: tab)))
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

    func dispose() {
        backObservation = nil
        webView?.stopLoading()
        webView?.navigationDelegate = nil
        webView?.configuration.userContentController.removeScriptMessageHandler(forName: "wontonBridge")
        webView = nil
        action = nil
    }
}

struct TabWebView: UIViewRepresentable {
    let webView: WKWebView
    /// 被底栏、Home 条挡住的底边：页面照样画在后面，但固定定位的元素和滚动的终点让开这块。顶边不用它，见 TabPageView.webFrame。
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
