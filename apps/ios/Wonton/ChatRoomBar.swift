import SwiftUI
import WontonClient

/// 主对话的顶栏（ADR-0109 第 9 节）：左边返回，中间一枚胶囊写名字和状态行（“正在输入…”也在这里），右边圆头像；
/// 几样都是浮在消息上的玻璃（ADR-0107），栏本身不垫底，消息从下面滚过去。
/// 点头像进对方的资料页；名字胶囊只显示名字和状态，不再另做一个入口。通话、旁聊这两个房间入口排在头像左边。面板的入口在资料页里（ADR-0052）。
struct ChatRoomBar<Trailing: View>: View {
    let title: String
    let avatar: AnyView?
    /// 有人正在输入时状态行写的字（ADR-0109 第 8 节）；没人输入为空。
    var typing: String?
    /// 点头像去的地方；没有资料页的房间为空，这时头像只是显示。
    var openProfile: (() -> Void)?
    @ViewBuilder let trailing: Trailing
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.dismiss) private var dismiss
    @Environment(\.wontonContentRoot) private var isContentRoot

    var body: some View {
        VStack(spacing: 0) {
            ChatBarLayout {
                if isContentRoot {
                    Color.clear.frame(width: 0, height: 0)
                } else {
                    ChatGlassButton(icon: .back, label: String(localized: .Design.designBack)) { dismiss() }
                }
                titleCapsule
                HStack(spacing: WontonTheme.spacing.s4) {
                    trailing
                    avatarView
                }
            }
            .chatGlassCluster()
            .padding(.horizontal, WontonTheme.spacing.s4)
            .frame(minHeight: WontonTheme.size.topBar)
            if let line = status.line {
                line
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.bottom, WontonTheme.spacing.s8)
            }
        }
    }

    /// 状态行：有人输入时写谁在输入，否则写连接状态；连接正常又没人输入时只剩名字。
    private var titleCapsule: some View {
        VStack(spacing: 0) {
            Text(title)
                .wontonText(WontonTheme.typography.barTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("roomTopBarTitle")
            if let typing {
                Text(typing)
                    .wontonText(WontonTheme.typography.barStatus)
                    .foregroundStyle(Color(colors.primary))
                    .lineLimit(1)
                    .accessibilityIdentifier("barActivity")
            } else if status.line == nil, let subtitle = status.subtitle {
                Text(subtitle)
                    .wontonText(WontonTheme.typography.barStatus)
                    .foregroundStyle(Color(colors.textSecondary))
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.chatControl)
        // 不包成按钮：包了以后名字和状态行在无障碍树里并成一个元素。头像是唯一的资料页入口，读屏也从头像那个按钮进。
        .chatGlass(in: Capsule())
    }

    @ViewBuilder private var avatarView: some View {
        if let avatar {
            let image = avatar
                .frame(width: WontonTheme.size.chatControl, height: WontonTheme.size.chatControl)
                .clipShape(Circle())
                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
            if let openProfile {
                Button(action: openProfile) { image.contentShape(Rectangle()) }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text(.BotProfile0.botProfileTitle))
                    .accessibilityIdentifier("roomTopBarAvatar")
            } else {
                image
                    .accessibilityElement(children: .ignore)
                    .accessibilityHidden(true)
            }
        }
    }
}

/// 顶栏的三段：左、中、右。中间那一段在整条栏里居中，两边的东西宽了就收窄（字截断）；窄到 `minimum` 还放不下，
/// 才退到两边之间的空里居中。
private struct ChatBarLayout: Layout {
    var spacing: CGFloat = WontonTheme.spacing.s4

    /// 居中时标题胶囊至少这么宽。
    private static let minimum = WontonTheme.size.chatControl * 2

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let sizes = subviews.map { $0.sizeThatFits(.unspecified) }
        let natural = sizes.reduce(CGFloat.zero) { $0 + $1.width } + spacing * 2
        return CGSize(width: proposal.width ?? natural, height: sizes.map(\.height).max() ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 3 else { return }
        let leading = subviews[0].sizeThatFits(.unspecified)
        let trailing = subviews[2].sizeThatFits(.unspecified)
        subviews[0].place(
            at: CGPoint(x: bounds.minX, y: bounds.midY), anchor: .leading, proposal: ProposedViewSize(leading)
        )
        subviews[2].place(
            at: CGPoint(x: bounds.maxX, y: bounds.midY), anchor: .trailing, proposal: ProposedViewSize(trailing)
        )
        let gap = max(0, bounds.width - leading.width - trailing.width - spacing * 2)
        let balanced = max(0, bounds.width - max(leading.width, trailing.width) * 2 - spacing * 2)
        let ideal = subviews[1].sizeThatFits(.unspecified).width
        let centered = min(ideal, balanced) >= min(ideal, Self.minimum)
        let width = min(ideal, centered ? balanced : gap)
        let center = centered ? bounds.midX : bounds.minX + leading.width + spacing + gap / 2
        subviews[1].place(
            at: CGPoint(x: center, y: bounds.midY), anchor: .center,
            proposal: ProposedViewSize(width: width, height: bounds.height)
        )
    }
}

/// 顶栏和输入栏后面垫的一层：材质加聊天背景的底色，朝消息的那一头在 `fade` 这么长的一段里淡到没有，
/// 看不出一条栏的边。消息滚到它底下就淡出去，玻璃上的字不和消息的字搅在一起。
struct ChatEdgeFade: View {
    let edge: VerticalEdge
    var fade: CGFloat = WontonTheme.spacing.s24
    @Environment(\.chatPalette) private var palette

    var body: some View {
        ZStack {
            Rectangle().fill(.regularMaterial)
            palette.wallBase.opacity(WontonTheme.ratio.secondaryContent)
        }
        .mask {
            VStack(spacing: 0) {
                if edge == .top { Color.black }
                LinearGradient(
                    colors: [.black, .clear],
                    startPoint: edge == .top ? .top : .bottom, endPoint: edge == .top ? .bottom : .top
                )
                .frame(height: fade)
                if edge == .bottom { Color.black }
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}
