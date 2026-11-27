import SwiftUI
import WontonClient

/// widget 页（ADR-0096）里驱动发给页面的消息：页面开始加载后订阅，每一段脚本原样注入同一个页面的主框架，
/// 壳不解析报文。页面发来的 widget 报文照常经 `wontonBridge` 交给核心，应答也从这条订阅回来。
@MainActor
final class TabWidgetChannel {
    private var subscription: TabWidgetSubscription?
    private var task: Task<Void, Never>?

    /// 每次加载是一次新的 widget 会话：上一条订阅先停。驱动还没起来时核心报未就绪，交给调用方显示。
    func start(page: PlacedTab, core: ClientCore, into session: TabWebSession) throws {
        stop()
        let subscription = try core.subscribeTabWidget(page: page)
        self.subscription = subscription
        task = Task { [weak session] in
            // 订阅返回空：widget 会话结束了（页面关了、换了房间、退出登录、重新加载），核心会跟着收页面或给新地址。
            while let script = await subscription.next() {
                guard !Task.isCancelled, let session, let view = session.webView else { return }
                session.evaluate(script, in: view)
            }
        }
    }

    /// 壳自己先拆页面时取消（Swift 的 Task 取消不会传到核心）。
    func stop() {
        subscription?.cancel()
        subscription = nil
        task?.cancel()
        task = nil
    }
}

extension TabRef {
    var isWidget: Bool {
        if case .widget = self { return true }
        return false
    }
}

/// 通话页上方的一条轻提示：只连上了公共中继，照样能打（ADR-0096 第 4 节）。
struct CallRelayHint: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            WontonIcon.connection.view(size: WontonTheme.size.iconSmall)
            Text(.Tab1.tabCallRelayHint)
                .wontonText(WontonTheme.typography.supporting)
                .fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(Color(colors.textPrimary))
        .padding(WontonTheme.spacing.s8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: WontonTheme.radii.control))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("callRelayHint")
        .allowsHitTesting(false)
    }
}
