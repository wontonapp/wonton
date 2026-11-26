import SwiftUI
import WontonClient

/// 网页右上角的胶囊：聊天页下拉面板（ADR-0052）、公网网页和内容窗口上的出口。别的整页不放（见 `TabPageView.hasCapsule`）。
struct TabPageCapsule: View {
    let page: TabPage
    let tab: TabRef
    let placed: PlacedTab
    let pages: PagesModel
    let session: TabWebSession
    let takeover: PageTakeover?
    let details: () -> Void
    /// 面板收着时不显示（ADR-0052），也不挡点按。
    let shown: Bool
    /// 在聊天页的下拉面板里（ADR-0052）：退出按钮关掉面板，菜单多一项“编辑面板”。
    let panel: TabPanelMode?
    /// 关掉这一页（面板外）。
    let close: () -> Void
    /// 关掉这一页：面板里摘掉面板项，整页里弹栈。菜单的“返回”判定要关页时用它。
    let closePage: () -> Void
    @Environment(\.wontonColors) private var colors

    /// 胶囊那一行的高度。带胶囊的整页，网页框从状态栏加这一行下面开始。
    static var rowHeight: CGFloat { WontonTheme.size.target }

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
            }
            .frame(height: Self.rowHeight)
            // 胶囊占着右边：连接原因单独一行，短句也不许挤成省略号（#149）。
            PageStatusLine()
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
                    Button(TabText.back) { session.backOrClose(onClosePage: closePage) }
                }
                if let takeover { TakeoverButton(takeover: takeover) }
                Button(TabText.reload) { pages.send(.pages(action: .reloadTab(page: placed))) }
                if !tab.isWidget { ContentWindowButton(target: .tab(tab: tab)) }
                if let panel {
                    Button(TabText.editPanel, action: panel.edit)
                } else {
                    // 钉住正在看的这页到聊天页的下拉面板（ADR-0052）；插件页再加进“我的应用”。
                    // widget 页只能从房间里打开，不能钉进面板（ADR-0096）。
                    if let url = session.webView?.url, !tab.isWidget {
                        Button(TabText.pinToPanel) {
                            pages.send(.pages(action: .pinCurrentPage(
                                url: url.absoluteString, title: session.webView?.title,
                                to: .panel(scope: .main)
                            )))
                        }
                    }
                    if case .plugin = tab {
                        Button(TabText.addToMyApps) {
                            pages.send(.pages(action: .addMyApp(app: .page(tab: tab), title: nil, index: nil)))
                        }
                        Button(TabText.details, action: details)
                    }
                }
            } label: {
                glyph(.more)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(TabText.more)
            .accessibilityIdentifier("pageMore")
            Rectangle()
                .fill(Color(colors.separator))
                .frame(width: WontonTheme.size.hairline, height: WontonTheme.size.iconSmall)
            trailing
        }
        .frame(height: WontonTheme.size.smallButton)
        .wontonGlassSurface(in: Capsule())
    }

    private var trailing: some View {
        Button {
            if let panel { panel.close() } else { close() }
        } label: { glyph(.leave) }
            .buttonStyle(.plain)
            .accessibilityLabel(TabText.close)
            // 界面测试按它认胶囊的退出：聊天页上别的“关闭”（提示条）同名。
            .accessibilityIdentifier("pageClose")
    }

    private var ink: Color { Color(colors.textPrimary) }

    /// 公网网页标出来源网站的域名，和插件页面一眼分开（ADR-0052）。
    private var sourceHost: String? {
        guard case let .url(value) = tab else { return nil }
        return URL(string: value)?.host()
    }

    private func glyph(_ icon: WontonIcon) -> some View {
        icon.view(size: WontonTheme.size.iconSmall)
            .foregroundStyle(ink)
            .frame(width: WontonTheme.size.target, height: WontonTheme.size.smallButton)
            .contentShape(Rectangle())
    }
}
