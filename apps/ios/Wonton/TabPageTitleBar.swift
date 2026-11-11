import SwiftUI
import WontonClient

/// 带栏的插件页顶上的标题栏。全屏时没有它，换成右上角的胶囊（TabPageCapsule）。
struct TabPageTitleBar: View {
    let page: TabPage
    let session: TabWebSession
    let takeover: PageTakeover?
    let isOverlay: Bool
    /// 铺满的页面画在标题栏后面，垫一层材质，字才读得清；没适配的页面从标题栏下面开始，不垫底色。
    let material: Bool
    let enterFullscreen: () -> Void
    let close: () -> Void
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status
    /// 两边按钮的宽度：接管按钮带文字，宽度不定。标题两边让出较宽的那边，居中且过长时截断，不和按钮叠在一起。
    @State private var leadingWidth: CGFloat = 0
    @State private var trailingWidth: CGFloat = 0

    var body: some View {
        VStack(spacing: 0) {
            ZStack {
                VStack(spacing: WontonTheme.spacing.s4) {
                    Text(page.title)
                        .wontonText(WontonTheme.typography.itemTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                        .lineLimit(1)
                        .padding(.horizontal, max(leadingWidth, trailingWidth) + WontonTheme.spacing.s8)
                        .accessibilityAddTraits(.isHeader)
                    if case .sharedApp = session.tab, let owner = status.subtitle {
                        Text(owner).wontonText(WontonTheme.typography.caption)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
                HStack(spacing: 0) {
                    HStack(spacing: 0) {
                        if session.canGoBack {
                            WontonIconButton(icon: .back, label: TabText.back) { session.webView?.goBack() }
                        }
                    }
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { leadingWidth = $0 }
                    Spacer(minLength: 0)
                    HStack(spacing: 0) {
                        if let takeover { TakeoverButton(takeover: takeover) }
                        WontonIconButton(icon: .expand, label: TabText.fullscreen, action: enterFullscreen)
                        if isOverlay {
                            WontonIconButton(icon: .close, label: TabText.close, action: close)
                        }
                    }
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { trailingWidth = $0 }
                }
                .padding(.horizontal, WontonTheme.spacing.s4)
            }
            .frame(minHeight: WontonTheme.size.topBar)
            // 工具按钮占着右边，标题只剩一点：连接原因单独一行，短句也不许挤成省略号（#149）。
            if let line = status.line {
                line.padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.bottom, WontonTheme.spacing.s8)
            } else if let sentence = status.subtitle {
                Text(sentence)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.bottom, WontonTheme.spacing.s8)
            }
        }
        .background {
            if material { Rectangle().fill(.ultraThinMaterial).ignoresSafeArea(edges: .top) }
        }
    }
}
