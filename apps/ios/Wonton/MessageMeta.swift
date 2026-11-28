import SwiftUI
import WontonClient

/// 气泡右下角常驻的一行（ADR-0109 第 3 节）：“已编辑”、时间，自己的消息后面跟钟或勾。
/// 勾和钟是系统自带的 SF Symbols。
struct MessageMeta: View {
    let message: TimelineMessage
    /// 压在纯图片上：垫一枚半透明深色胶囊，字用白色。
    var onMedia = false
    /// 正文末尾的占位文字已经把“已编辑”和时间念过一遍时，这里的字不再念，只留状态。
    var textIsSpoken = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            Group {
                if message.edited { Text(.Message2.messageEdited) }
                Text(TimelineRow.time(message.timestampMs))
            }
            .accessibilityHidden(textIsSpoken)
            status
        }
        .wontonText(WontonTheme.typography.messageMeta)
        .foregroundStyle(ink)
        .lineLimit(1)
        .fixedSize()
        .padding(.horizontal, onMedia ? WontonTheme.spacing.s8 : 0)
        .padding(.vertical, onMedia ? WontonTheme.spacing.s4 / 2 : 0)
        .background {
            if onMedia { Capsule().fill(Color.black.opacity(WontonTheme.ratio.scrim)) }
        }
        #if DEBUG
        // 走查用：时间的字在无障碍树里是藏起来的（正文已经念过），界面测试靠这个看不见的记号量它的位置。只在 Debug 包里挂。
        .overlay {
            Color.clear.accessibilityElement().accessibilityIdentifier("messageMeta")
        }
        .overlay {
            if message.edited {
                Color.clear.accessibilityElement().accessibilityIdentifier("messageEdited")
            }
        }
        #endif
    }

    private var ink: Color {
        if onMedia { return .white }
        return Color(message.isOwn ? colors.onPrimary : colors.textSecondary)
    }

    /// 发送失败不在这里：红色感叹号画在气泡旁边。更新的核心给出的不认识的状态不画。
    @ViewBuilder private var status: some View {
        if message.isOwn {
            switch message.sendState {
            case .sending:
                WontonIcon.clock.view(size: WontonTheme.size.iconTiny)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text(.Message2.messageSending))
                    .accessibilityIdentifier("messageStatusSending")
            case .failed:
                EmptyView()
            case .sent:
                switch message.status {
                case .sent:
                    WontonIcon.check.view(size: WontonTheme.size.iconTiny)
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel(Text(.Message2.messageDelivered))
                        .accessibilityIdentifier("messageStatusSent")
                case .read:
                    WontonIcon.read.view(size: WontonTheme.size.iconTiny)
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel(Text(.Message2.messageRead))
                    .accessibilityIdentifier("messageStatusRead")
                case .other, nil:
                    EmptyView()
                }
            }
        }
    }

    /// 正文末尾留给这一行的透明占位：放得下就和末行同排，放不下占位自己换行，这一行就落在新的一行。
    /// 开头是普通空格（能在这里折行），其余用不折行的空格连着；图标的位置也用不折行的空格占。
    static func reserve(for message: TimelineMessage) -> String {
        var parts: [String] = []
        if message.edited { parts.append(String(localized: .Message2.messageEdited)) }
        parts.append(TimelineRow.time(message.timestampMs))
        if message.isOwn { parts.append(String(repeating: "\u{00A0}", count: iconReserve)) }
        let glued = parts.joined(separator: "\u{00A0}").replacingOccurrences(of: " ", with: "\u{00A0}")
        return " \u{00A0}\u{00A0}" + glued
    }

    /// 两个勾大约占几个不折行空格的宽（小号字里一个空格约四分之一个字宽）。
    private static let iconReserve = 8
}
