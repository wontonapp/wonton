import SwiftUI

/// 第一次整页打开应用时盖在页面上的引导：整页顶上不放壳的控件（ADR-0095），出口只有左边缘右滑，要教一次。
struct PageExitHint: View {
    let dismiss: () -> Void
    @State private var swiped = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    private let travel: CGFloat = 140

    var body: some View {
        ZStack {
            // 遮罩接住点按：引导还在时不把点按漏给下面的网页。
            Color.black.opacity(0.6)
                .contentShape(Rectangle())
                .accessibilityHidden(true)
            VStack(spacing: WontonTheme.spacing.s24) {
                gesture
                Text(.Tab0.tabExitHintText)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                Button(String(localized: .Tab1.tabExitHintGotIt), action: dismiss)
                    .buttonStyle(WontonButtonStyle())
                    .accessibilityIdentifier("pageExitHintDismiss")
            }
            .padding(WontonTheme.spacing.s32)
        }
        .ignoresSafeArea(.container)
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isModal)
        .accessibilityIdentifier("pageExitHint")
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeInOut(duration: 1.4).delay(0.3).repeatForever(autoreverses: false)) { swiped = true }
        }
    }

    /// 屏幕左边缘的一道亮线，一只手指从它往右滑。减弱动态效果时手指停在滑出去的位置，只留箭头示意方向。
    private var gesture: some View {
        HStack(spacing: 0) {
            Capsule()
                .fill(.white)
                .frame(width: 4, height: 72)
            ZStack(alignment: .leading) {
                Image(systemName: "arrow.right")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.5))
                    .offset(x: travel / 2 - 11)
                Image(systemName: "hand.point.up.left.fill")
                    .font(.system(size: 44))
                    .foregroundStyle(.white)
                    .offset(x: reduceMotion || swiped ? travel : 0, y: 16)
                    .opacity(!reduceMotion && swiped ? 0 : 1)
            }
            .frame(width: travel + 56, height: 88, alignment: .leading)
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.leading, -WontonTheme.spacing.s32)
        .accessibilityHidden(true)
    }
}

/// 引导看过没有，只记在本机：点了“知道了”就不再出现。
struct PageExitHintMemory {
    /// 没有存储的一份当作看过了。
    private let defaults: UserDefaults?

    var seen: Bool { defaults?.bool(forKey: Self.storageKey) ?? true }

    func remember() {
        defaults?.set(true, forKey: Self.storageKey)
    }

    static var current: PageExitHintMemory {
        #if DEBUG
        // 界面测试每次启动用一份空数据（`WONTON_DEV_DATA_SUFFIX`），但偏好是整台模拟器共用的：默认当作看过，
        // 引导不挡别的用例；要测引导的用例设 `WONTON_UI_PAGE_EXIT_HINT`，按数据后缀各记各的。
        let environment = ProcessInfo.processInfo.environment
        if let suffix = environment["WONTON_DEV_DATA_SUFFIX"] {
            guard environment["WONTON_UI_PAGE_EXIT_HINT"] == "1" else { return PageExitHintMemory(defaults: nil) }
            return PageExitHintMemory(defaults: UserDefaults(suiteName: "pageExitHint.\(suffix)"))
        }
        #endif
        return PageExitHintMemory(defaults: .standard)
    }

    private static let storageKey = "pageExitHintSeen"
}
