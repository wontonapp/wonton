import SwiftUI
import WontonClient

/// “家”：和家里服务器的连接状态，以及插件网页入口。只渲染核心给出的 ViewModel。
struct HomeView: View {
    @Bindable var model: HomeModel
    @State private var draftTicket = ""
    @State private var isShowingPage = false

    var body: some View {
        NavigationStack {
            Form {
                Section("本机") {
                    LabeledContent("设备 ID") {
                        Text(model.viewModel.deviceId)
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                    }
                }
                Section("家里的服务器") {
                    LabeledContent("状态", value: statusText(model.viewModel.status))
                    if let path = model.viewModel.path {
                        LabeledContent("路径", value: "\(path.relay ? "中继" : "直连") · \(path.rttMs) ms")
                    }
                    if let proxy = model.viewModel.proxy {
                        LabeledContent("本地代理", value: "\(proxy.host):\(proxy.port)")
                    }
                }
                if let page = page {
                    Section {
                        Button("打开 \(page.url.host() ?? page.url.absoluteString)") { isShowingPage = true }
                    }
                }
                Section("连接信息") {
                    TextField("粘贴 dev.sh ticket 的输出", text: $draftTicket, axis: .vertical)
                        .font(.caption.monospaced())
                        .lineLimit(3...6)
                    Button("连接") { model.send(.connectHome(ticket: draftTicket)) }
                        .disabled(draftTicket.isEmpty)
                }
            }
            .navigationTitle("家")
            .onChange(of: page?.url, initial: true) { _, url in
                // 开发时 WONTON_DEV_OPEN_WEB=1：本地代理就绪后自动打开网页，真机上不用点按也能测。
                #if DEBUG
                if url != nil, ProcessInfo.processInfo.environment["WONTON_DEV_OPEN_WEB"] != nil {
                    isShowingPage = true
                }
                #endif
            }
            .fullScreenCover(isPresented: $isShowingPage) {
                if let page = page {
                    PluginPage(url: page.url, proxy: page.proxy) { isShowingPage = false }
                }
            }
            .alert("操作被拒绝", isPresented: isShowingRejection, presenting: model.rejection) { _ in
                Button("好") {}
            } message: { reason in
                Text(reason)
            }
        }
        .task { await model.observe() }
    }

    /// 网页入口和本地代理都就绪时才能打开网页。
    private var page: (url: URL, proxy: ProxySettings)? {
        guard let text = model.viewModel.webUrl, let url = URL(string: text),
              let proxy = model.viewModel.proxy else { return nil }
        return (url, proxy)
    }

    private var isShowingRejection: Binding<Bool> {
        Binding(
            get: { model.rejection != nil },
            set: { isShowing in
                if !isShowing { model.rejection = nil }
            }
        )
    }

    private func statusText(_ status: HomeStatus) -> String {
        switch status {
        case .notConfigured:
            return "还没有连接信息"
        case .connecting:
            return "连接中"
        case .connected:
            return "已连接"
        case let .disconnected(reason, message):
            switch reason {
            case .unauthorized:
                return "服务端不认这台设备，请主人先登记它"
            case .transportUnavailable, .upstreamUnavailable, .protocol:
                return message
            }
        }
    }
}

/// 全屏的插件网页，带一个关闭按钮。
private struct PluginPage: View {
    let url: URL
    let proxy: ProxySettings
    let close: () -> Void

    var body: some View {
        NavigationStack {
            PluginWebView(url: url, proxy: proxy)
                .ignoresSafeArea(edges: .bottom)
                .navigationTitle(url.host() ?? "")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    Button("关闭", action: close)
                }
        }
    }
}
