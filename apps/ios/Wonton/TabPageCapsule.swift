import SwiftUI
import WontonClient

/// 全屏插件页右上角的胶囊，会自动藏起来（见 TabPageView）。带栏时用标题栏（TabPageTitleBar）。
struct TabPageCapsule: View {
    let page: TabPage
    let tab: TabRef
    let tabs: TabsModel
    let session: TabWebSession
    let takeover: PageTakeover?
    let details: () -> Void
    /// 藏起来时不挡点按，点到的是下面的页面。
    let shown: Bool
    /// 碰了胶囊，重新计时再藏。
    let touched: () -> Void
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s8) {
                if let title = status.title {
                    Text(title)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textPrimary))
                        .padding(.horizontal, WontonTheme.spacing.s12)
                        .padding(.vertical, WontonTheme.spacing.s8)
                        .background(.regularMaterial, in: Capsule())
                }
                Spacer(minLength: 0)
                capsule
                    .opacity(shown ? 1 : 0)
                    .allowsHitTesting(shown)
                    .accessibilityHidden(!shown)
                    .animation(.easeInOut(duration: 0.2), value: shown)
                    .simultaneousGesture(TapGesture().onEnded(touched))
            }
            .frame(height: WontonTheme.size.target)
            if let line = status.line {
                line.padding(WontonTheme.spacing.s12)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: WontonTheme.radii.control))
            }
        }
    }

    private var capsule: some View {
        HStack(spacing: 0) {
            Menu {
                if session.canGoBack {
                    Button(TabText.back) { session.webView?.goBack() }
                }
                if let takeover { TakeoverButton(takeover: takeover) }
                Button(TabText.reload) { tabs.send(.reloadTab(page: .bar(tab: tab))) }
                Toggle(TabText.alwaysFullscreen, isOn: Binding(
                    get: { page.opensFullscreen },
                    set: { tabs.send(.setTabDisplay(tab: tab, display: $0 ? .fullscreen : .standard)) }
                ))
                Button(TabText.details, action: details)
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
        Button { tabs.send(.exitFullscreen(tab: tab)) } label: { glyph(.leave) }
            .buttonStyle(.plain)
            .accessibilityLabel(TabText.close)
    }

    private var ink: Color { .white }

    private func glyph(_ icon: WontonIcon) -> some View {
        icon.view(size: WontonTheme.size.iconSmall)
            .foregroundStyle(ink)
            .shadow(color: .black.opacity(0.4), radius: 1)
            .frame(width: WontonTheme.size.target, height: WontonTheme.size.smallButton)
            .contentShape(Rectangle())
    }
}
