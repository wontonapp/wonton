import SwiftUI
import WontonClient

/// 全屏插件页右上角的胶囊，会自动藏起来（见 TabPageView）。带栏时用标题栏（TabPageTitleBar）。
struct TabPageCapsule: View {
    let page: TabPage
    let tab: TabRef
    let placed: PlacedTab
    let tabs: TabsModel
    let session: TabWebSession
    let takeover: PageTakeover?
    let details: () -> Void
    /// 藏起来时不挡点按，点到的是下面的页面。
    let shown: Bool
    /// 碰了胶囊，重新计时再藏。
    let touched: () -> Void
    /// 在聊天页的下拉面板里（ADR-0052）：退出按钮关掉面板，菜单多一项“编辑面板”，没有“总是全屏”。
    let panel: TabPanelMode?
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s8) {
                Spacer(minLength: 0)
                capsule
                    // 来源域名的字号封顶：最大的辅助字号下胶囊不被撑出屏幕，退出一直点得到。
                    .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                    .opacity(shown ? 1 : 0)
                    .allowsHitTesting(shown)
                    .accessibilityHidden(!shown)
                    .animation(.easeInOut(duration: 0.2), value: shown)
                    .simultaneousGesture(TapGesture().onEnded(touched))
            }
            .frame(height: WontonTheme.size.target)
            // 胶囊占着右边：连接原因单独一行，短句也不许挤成省略号（#149）。
            if let line = status.line {
                line.padding(WontonTheme.spacing.s12)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: WontonTheme.radii.control))
            } else if let sentence = status.subtitle {
                Text(sentence)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(WontonTheme.spacing.s12)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: WontonTheme.radii.control))
            }
        }
    }

    private var capsule: some View {
        HStack(spacing: 0) {
            if let host = sourceHost {
                HStack(spacing: WontonTheme.spacing.s4) {
                    WontonIcon.globe.view(size: WontonTheme.size.iconTiny)
                    Text(host)
                        .wontonText(WontonTheme.typography.caption)
                        .lineLimit(1)
                        .frame(maxWidth: 130, alignment: .leading)
                }
                .foregroundStyle(ink)
                .padding(.leading, WontonTheme.spacing.s8)
            }
            Menu {
                if session.canGoBack {
                    Button(TabText.back) { session.webView?.goBack() }
                }
                if let takeover { TakeoverButton(takeover: takeover) }
                Button(TabText.reload) { tabs.send(.reloadTab(page: placed)) }
                if let panel {
                    Button(TabText.editPanel, action: panel.edit)
                } else {
                    Toggle(TabText.alwaysFullscreen, isOn: Binding(
                        get: { page.opensFullscreen },
                        set: { tabs.send(.setTabDisplay(tab: tab, display: $0 ? .fullscreen : .standard)) }
                    ))
                }
                pin
                if hasDetails { Button(TabText.details, action: details) }
            } label: {
                glyph(.more)
            }
            .accessibilityLabel(TabText.more)
            Rectangle()
                .fill(ink.opacity(0.2))
                .frame(width: WontonTheme.size.hairline, height: WontonTheme.size.iconSmall)
            trailing
        }
        .frame(height: WontonTheme.size.smallButton)
        // 照微信的透明胶囊：一层淡黑、不模糊，白色图标带一圈淡阴影；不跟页面换深浅。
        // 浅色页面上看得出灰色的胶囊，深色页面上看得出白色图标。
        .background(Color.black.opacity(0.2), in: Capsule())
    }

    private var trailing: some View {
        Button {
            if let panel { panel.close() } else { tabs.send(.exitFullscreen(tab: tab)) }
        } label: { glyph(.leave) }
            .buttonStyle(.plain)
            .accessibilityLabel(TabText.close)
    }

    private var ink: Color { .white }

    /// 公网网页没有插件详情。
    private var hasDetails: Bool {
        switch tab {
        case .plugin, .pluginPath: true
        default: false
        }
    }

    /// 公网网页标出来源网站的域名，和插件页面一眼分开（ADR-0052）。
    private var sourceHost: String? {
        guard case let .url(value) = tab else { return nil }
        return URL(string: value)?.host()
    }

    /// 钉住正在看的这一页：面板里的钉到底栏，底栏的钉到主面板（ADR-0052）。
    @ViewBuilder private var pin: some View {
        if let url = session.webView?.url {
            Button(panel == nil ? TabText.pinToPanel : TabText.pinToBar) {
                let target: PinTarget = panel == nil ? .panel(scope: .main) : .bar
                tabs.send(.pinCurrentPage(
                    url: url.absoluteString, title: session.webView?.title, to: target, replacing: nil
                ))
            }
        }
    }

    private func glyph(_ icon: WontonIcon) -> some View {
        icon.view(size: WontonTheme.size.iconSmall)
            .foregroundStyle(ink)
            .shadow(color: .black.opacity(0.4), radius: 1)
            .frame(width: WontonTheme.size.target, height: WontonTheme.size.smallButton)
            .contentShape(Rectangle())
    }
}
