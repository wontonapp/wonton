import SwiftUI
import WontonClient

/// “家”：还没配对时是配对页，配上以后是和家里服务器的连接状态。只渲染核心给出的 ViewModel。
struct HomeView: View {
    @Bindable var model: HomeModel
    /// 开发时经 `WONTON_DEV_OPEN_WEB` 传入的插件网页，本地代理就绪后自动打开，真机上不用点按也能测。
    var devWebURL: URL?
    @State private var isShowingPage = false
    @State private var isPairing = false
    @State private var isInviting = false

    var body: some View {
        NavigationStack {
            Group {
                if case .notConfigured = model.viewModel.status {
                    PairingView(model: model)
                } else {
                    connection
                }
            }
            .navigationTitle("家")
            .onChange(of: model.viewModel.proxy, initial: true) { _, proxy in
                if devWebURL != nil, proxy != nil { isShowingPage = true }
            }
            .fullScreenCover(isPresented: $isShowingPage) {
                if let url = devWebURL, let proxy = model.viewModel.proxy {
                    PluginPage(url: url, proxy: proxy) { isShowingPage = false }
                }
            }
            .sheet(isPresented: $isPairing) {
                NavigationStack {
                    PairingView(model: model) { isPairing = false }
                        .navigationTitle("重新配对")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("关闭") { isPairing = false }
                            }
                        }
                }
            }
            .sheet(isPresented: $isInviting) {
                InviteView(model: model) { isInviting = false }
            }
            .alert("操作被拒绝", isPresented: isShowingRejection, presenting: model.rejection) { _ in
                Button("好") {}
            } message: { reason in
                Text(reason)
            }
        }
        .task { await model.observe() }
    }

    private var connection: some View {
        Form {
            Section {
                LabeledContent("状态", value: statusText(model.viewModel.status))
                if let path = model.viewModel.path {
                    LabeledContent("路径", value: "\(path.relay ? "中继" : "直连") · \(path.rttMs) ms")
                }
            }
            Section {
                Button("添加设备") { isInviting = true }
                Button("重新配对") { isPairing = true }
            }
        }
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
            return "还没有配对"
        case .connecting:
            return "连接中"
        case .connected:
            return "已连接"
        case let .disconnected(reason, message):
            switch reason {
            case .unauthorized:
                return "家里的服务器不认这台设备，请重新扫码配对"
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
