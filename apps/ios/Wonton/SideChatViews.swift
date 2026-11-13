import SwiftUI
import WontonClient

extension SideChatState {
    /// 旁聊状态的文字（ADR-0093 §4 的表）。
    var text: String {
        switch self {
        case .open: String(localized: .Chat.sideChatStateOpen)
        case .done: String(localized: .Chat.sideChatStateDone)
        case .failed: String(localized: .Chat.sideChatStateFailed)
        case .ended: String(localized: .Chat.sideChatStateEnded)
        case .resultMissing: String(localized: .Chat.sideChatStateResultMissing)
        case .untracked: String(localized: .Chat.sideChatStateUntracked)
        case .other: String(localized: .Chat.sideChatStateOther)
        }
    }

    /// 这一轮失败了用醒目的颜色，其余是轻的说明。
    var isFailure: Bool { self == .failed }
}

/// 主对话里的旁聊卡片（ADR-0093 §6）：标题，下面一行“旁聊 · 3 条”和状态，有未读时右边一个角标。点了进旁聊。
/// 带标记的根占满一行，普通消息下面挂的随内容宽。
struct SideChatCardView: View {
    let card: SideChatCard
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            HStack(spacing: WontonTheme.spacing.s8) {
                WontonIcon.chat.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                    Text(card.title)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                        .lineLimit(1)
                    HStack(spacing: WontonTheme.spacing.s8) {
                        Text(.Chat.sideChatReplies(count: Int(card.replyCount)))
                        Text(card.state.text)
                            .foregroundStyle(Color(card.state.isFailure ? colors.danger : colors.textSecondary))
                    }
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                    .lineLimit(1)
                }
                Spacer(minLength: 0)
                if card.unreadCount > 0 {
                    Text(unreadLabel)
                        .wontonText(WontonTheme.typography.caption)
                        .monospacedDigit()
                        .foregroundStyle(Color(colors.accentInkOnFill))
                        .padding(.horizontal, WontonTheme.spacing.s8)
                        .padding(.vertical, WontonTheme.spacing.s4)
                        .accentFill(Capsule())
                        .accessibilityLabel(Text(.Chat.sideChatUnread(count: Int(card.unreadCount))))
                }
            }
            .padding(.horizontal, WontonTheme.spacing.s12)
            .padding(.vertical, WontonTheme.spacing.s8)
            .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
            .contentShape(RoundedRectangle(cornerRadius: WontonTheme.radii.card))
        }
        .buttonStyle(.plain)
    }

    /// 数字超过 99 就写“99+”。
    private var unreadLabel: String {
        let unread = Int(card.unreadCount)
        return unread > 99 ? "99+" : String(unread)
    }
}

/// 主对话里的旁聊结果行（ADR-0093 §6）：一行小字“旁聊「标题」：状态”，点了进旁聊。
struct SideChatResultLine: View {
    let title: String
    let state: SideChatState
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            Text(.Chat.sideChatResultLine(title, state.text))
                .wontonText(WontonTheme.typography.caption)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
                .foregroundStyle(Color(state.isFailure ? colors.danger : colors.textSecondary))
                .padding(.vertical, WontonTheme.spacing.s4)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// 旁聊页顶栏下面一行：这个旁聊的状态；AI 不回报结果时是一句轻提示（ADR-0093 §4）。
struct SideChatHeaderLine: View {
    let state: SideChatState
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(state.text)
            .wontonText(WontonTheme.typography.caption)
            .foregroundStyle(Color(state.isFailure ? colors.danger : colors.textSecondary))
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .padding(.vertical, WontonTheme.spacing.s4)
    }
}

/// “新建聊天”选的 AI 不支持旁聊、打开了主对话：顶上一句（ADR-0093 §7）。
struct NoSideChatsHint: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.info.view(size: WontonTheme.size.iconTiny)
            Text(.Chat.sideChatUnsupported)
        }
        .wontonText(WontonTheme.typography.caption)
        .foregroundStyle(Color(colors.textSecondary))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s4)
    }
}

/// 开新旁聊面板的内容（ADR-0093 §6）：主人写第一句，发送时核心先发根消息、再把这句放进旁聊，成功后
/// [onStarted] 拿到根的事件 ID 进旁聊页。没开成时面板留着写好的字，按类别说原因，配重试（§7）。
/// 发送中关掉面板不撤回：根发出去了，旁聊照样出现在主对话里。
struct NewSideChatFields: View {
    let model: ChatModel
    let roomId: String
    /// 面板的关闭回调（WontonSheet 给的）：先收起面板再开旁聊页，两次呈现不撞上。
    let close: (@escaping () -> Void) -> Void
    let onStarted: (String) -> Void
    @State private var firstMessage = ""
    @State private var sending = false
    @State private var failure: Problem?

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            TextField(String(localized: .Chat.sideChatPlaceholder), text: $firstMessage, axis: .vertical)
                .lineLimit(1...6)
                .textInputAutocapitalization(.sentences)
                .wontonField()
                .disabled(sending)
            if let failure {
                ProblemRow(problem: failure, retry: start)
            }
            Button(action: start) {
                Text(.Composer.composerSend).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .primary))
            .disabled(firstMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || sending)
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }

    private func start() {
        let body = firstMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !body.isEmpty, !sending else { return }
        sending = true
        failure = nil
        let core = model.core
        Task {
            defer { sending = false }
            do {
                let root = try await core.startSideChat(roomId: roomId, body: body)
                close { onStarted(root) }
            } catch {
                failure = error.problem
            }
        }
    }
}
