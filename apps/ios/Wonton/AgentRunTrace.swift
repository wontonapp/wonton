import SwiftUI
import WontonClient

/// 状态行下面那一行“N 步”（ADR-0128 第 4 节）：有清单或总结可看时是按钮，点了展开或收起；默认收起。
/// 没有可看的就只是一行字。
struct AgentRunSteps: View {
    let total: Int
    let expandable: Bool
    @Binding var expanded: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if expandable {
            Button {
                withAnimation(reduceMotion ? nil : .easeOut(duration: WontonTheme.motion.reduced)) {
                    expanded.toggle()
                }
            } label: {
                line.frame(minHeight: WontonTheme.size.chip).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("agentRun-steps")
        } else {
            line
        }
    }

    private var line: some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            Text(.AgentRun.agentRunSteps(total))
            if expandable {
                // 装饰：整行可点，尖角只表示能展开。
                WontonIcon.forward.view(size: WontonTheme.size.iconTiny)
                    .rotationEffect(.degrees(expanded ? 90 : 0))
                    .accessibilityHidden(true)
            }
        }
        .wontonText(WontonTheme.typography.caption)
        .foregroundStyle(Color(colors.textSecondary))
    }
}

/// 这一轮调过的工具，一项一行：按类别的图标加 Bot 写的说明（原样显示，不翻译）。进行中小转圈（减弱动态效果时不转），
/// 失败用错误色的标记，结果不确定用不确定的标记，完成不加标记。
struct AgentRunTrace: View {
    let calls: [AgentToolCall]
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            ForEach(calls, id: \.id) { call in
                HStack(spacing: WontonTheme.spacing.s8) {
                    Self.icon(call.kind).view(size: WontonTheme.size.iconTiny)
                        .accessibilityHidden(true)
                    Text(call.title)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                    mark(call.status)
                }
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("agentRun-trace")
    }

    @ViewBuilder private func mark(_ status: AgentToolStatus) -> some View {
        switch status {
        case .inProgress:
            if !reduceMotion {
                ProgressView().controlSize(.mini).tint(Color(colors.accentInk))
            }
        case .failed:
            WontonIcon.error.view(size: WontonTheme.size.iconTiny)
                .foregroundStyle(Color(colors.danger))
        case .uncertain:
            WontonIcon.help.view(size: WontonTheme.size.iconTiny)
                .foregroundStyle(Color(colors.warning))
        case .completed, .other:
            EmptyView()
        }
    }

    private static func icon(_ kind: AgentToolKind) -> WontonIcon {
        switch kind {
        case .read: .document
        case .edit: .compose
        case .delete: .trash
        case .move: .folder
        case .search: .search
        case .execute: .code
        case .think: .agent
        case .fetch: .globe
        case .other: .wrench
        }
    }
}
