import SwiftUI
import WontonClient

/// 房间顶栏的 widget 入口（ADR-0096）：一个插件是一个按钮，图标按清单的 `entry`；装了几个时出菜单，名字用插件名。
/// 房间里有人在通话时按钮换成带“通话中”的样子，点了就是加入。
struct RoomCallEntry: View {
    let widgets: [RoomWidget]
    let active: Bool
    let open: (RoomWidget) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Group {
            if widgets.count == 1, let widget = widgets.first {
                Button { open(widget) } label: { label(widget.entry) }
                    .accessibilityLabel(active ? activeText : widget.entryLabel)
            } else if let first = widgets.first {
                Menu {
                    ForEach(widgets, id: \.tab) { widget in
                        Button(widget.title) { open(widget) }
                    }
                } label: { label(first.entry) }
                    .accessibilityLabel(active ? activeText : first.entryLabel)
            }
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(active ? "roomCallActive" : "roomCall")
    }

    private var activeText: String { String(localized: .Chat.roomCallActive) }

    @ViewBuilder private func label(_ entry: WidgetEntry) -> some View {
        if active {
            HStack(spacing: WontonTheme.spacing.s4) {
                entry.icon(filled: true).view(size: WontonTheme.size.iconSmall)
                Text(activeText).wontonText(WontonTheme.typography.caption).lineLimit(1)
            }
            .foregroundStyle(Color(colors.success))
            .padding(.horizontal, WontonTheme.spacing.s8)
            .frame(minHeight: WontonTheme.size.smallButton)
            .background(Color(colors.success).opacity(0.14), in: Capsule())
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        } else {
            entry.icon(filled: false).view()
                .foregroundStyle(Color(colors.textPrimary))
                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                .contentShape(Rectangle())
        }
    }
}

private extension WidgetEntry {
    func icon(filled: Bool) -> WontonIcon {
        switch self {
        case .videoCall: filled ? .videoCallFill : .videoCall
        case .voiceCall: filled ? .voiceCallFill : .voiceCall
        case .other: filled ? .appsFill : .apps
        }
    }
}

private extension RoomWidget {
    /// 读屏的名字：通话按种类说，别的 widget 用插件名。
    var entryLabel: String {
        switch entry {
        case .videoCall: String(localized: .Chat.roomCallVideo)
        case .voiceCall: String(localized: .Chat.roomCallVoice)
        case .other: title
        }
    }
}
