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

/// 主对话上的旁聊盖层（ADR-0093）：下面的主对话继续订阅，被盖住时隐藏、读不到、摸不着、右滑不退出整个房间；
/// 上面用 `fullScreenCover` 单开一张全屏页：不能塞进房间自己的布局，房间被压进的栈里再套一层栈、
/// 内层在 ZStack 里量不出高度，整页会变空（iOS 26 模拟器实测）。
/// 盖着时同时按住外面的边缘返回手势（`keepsRoomOnSideChat`）：旁聊页那一层的根没有可退的页面。
struct SideChatCover<Main: View>: View {
    let model: ChatModel
    let plugins: PluginsModel
    let roomId: String
    let title: String
    @ViewBuilder let main: () -> Main
    /// 呈现中的旁聊根。模型里的旁聊一没（返回、旁聊自己结束、离开房间）盖层就从这里收，收完自己再清；
    /// 直接读模型的话收起动画还在跑、内容已经变空，滑下去的半秒是一张空白页。
    @State private var shown: Shown?

    private struct Shown: Identifiable {
        let id: String
    }

    var body: some View {
        let covered = model.sideChats[roomId] != nil
        main()
            .opacity(covered || shown != nil ? 0 : 1)
            .allowsHitTesting(covered || shown == nil)
            .accessibilityHidden(covered || shown != nil)
            .navigationBarBackButtonHidden(covered || shown != nil)
            // 侧栏被别处关掉时在这里清 `shown`；自己点返回清的是 `item`，内容留到 `onDismiss` 再收，转场里不抽走。
            .onChange(of: model.sideChats[roomId], initial: true) { _, root in
                if let root {
                    if shown?.id != root { shown = Shown(id: root) }
                } else {
                    shown = nil
                }
                blockOuterPop()
            }
            .onChange(of: shown?.id) { blockOuterPop() }
            .onDisappear { UINavigationController.keepsRoomOnSideChat = false }
            .fullScreenCover(
                item: $shown,
                onDismiss: {
                    shown = nil
                    blockOuterPop()
                },
                content: { shown in
                    NavigationStack {
                        ChatRoomView(
                            model: model, plugins: plugins, roomId: roomId, threadRoot: shown.id, title: title
                        )
                    }
                }
            )
    }

    private func blockOuterPop() {
        UINavigationController.keepsRoomOnSideChat = model.sideChats[roomId] != nil || shown != nil
    }
}

/// 主对话上的“开新旁聊”面板（ADR-0093 §6）：`model.composingSideChat == roomId` 时弹出，挂在房间内容上。
struct NewSideChatSheet: ViewModifier {
    let model: ChatModel
    let roomId: String
    /// 开成了的旁聊根：面板收完再开盖层，两次呈现不撞在同一个呈现时机上。
    @State private var started: String?

    func body(content: Content) -> some View {
        content.wontonSheet(
            isPresented: Binding(
                get: { model.composingSideChat == roomId },
                set: { if !$0 { model.composingSideChat = nil } }
            ),
            title: String(localized: .Chat.sideChatNew),
            afterDismiss: {
                guard let root = started else { return }
                started = nil
                model.openSideChat(root, in: roomId)
            },
            content: { close in
                NewSideChatFields(model: model, roomId: roomId, close: close) { root in
                    started = root
                }
            }
        )
    }
}

extension ChatRoomView {
    /// 旁聊页盖在这条主对话上（ADR-0093）：只挂在主对话实例上。
    var covered: Bool { threadRoot == nil && model.sideChats[roomId] != nil }

    /// 房间里有旁聊支持声明、能写：顶栏给开旁聊的按钮（ADR-0093 §1、§6）。
    var supportsSideChats: Bool {
        guard threadRoot == nil, let row = model.viewModel.rooms.first(where: { $0.roomId == roomId }) else {
            return false
        }
        return row.sideChats && !row.isReadOnly
    }
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
        .accessibilityIdentifier("sideChatCard.\(card.rootEventId)")
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
            .accessibilityIdentifier("sideChatHeader")
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

/// 旁聊输入栏上的一句：文件只能在主对话里发（照片这里直接发，ADR-0093 §5），右边一个关闭钮，底下带“打开主对话”直接过去。
struct SideChatAttachHint: View {
    let openMain: () -> Void
    let close: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                    WontonIcon.info.view(size: WontonTheme.size.iconTiny)
                        .padding(.top, WontonTheme.spacing.s4)
                    Text(.Composer.composerAttachOnlyMain)
                }
                Button(action: openMain) {
                    Text(.Composer.composerOpenMainChat).fontWeight(.medium)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("sideChatOpenMain")
            }
            .wontonText(WontonTheme.typography.caption)
            .foregroundStyle(Color(colors.textSecondary))
            .frame(maxWidth: .infinity, alignment: .leading)
            ChatGlassButton(icon: .close, label: String(localized: .Common.commonClose), action: close)
                .accessibilityIdentifier("sideChatHintClose")
        }
        .padding(WontonTheme.spacing.s12)
        .chatGlass(in: RoundedRectangle(cornerRadius: WontonTheme.radii.card, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("sideChatAttachHint")
    }
}

/// 开新旁聊面板的内容（ADR-0093 §6）：主人写第一句，发送时核心先发根消息、再把这句放进旁聊，成功后
/// [onStarted] 拿到根的事件 ID 进旁聊页。没开成时面板留着写好的字，按类别说原因，配重试（§7）。
/// 发送中关掉面板不撤回：根发出去了，旁聊照样出现在主对话里。
struct NewSideChatFields: View {
    let model: ChatModel
    let roomId: String
    /// 面板的关闭回调（WontonSheet 给的）：收起面板，面板的 `afterDismiss` 再开旁聊页。
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
                .accessibilityIdentifier("sideChatFirstMessage")
            if let failure {
                ProblemRow(problem: failure, retry: start)
            }
            Button(action: start) {
                Text(.Composer.composerSend).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .primary))
            .disabled(firstMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || sending)
            .accessibilityIdentifier("sideChatStart")
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
