import Photos
import PhotosUI
import SwiftUI
import WontonClient

/// 聊天页底部的输入栏。主对话里相册、文件和粘贴进来的东西都先进附件区（ADR-0094），只有发送按钮把字和附件
/// 一起发出去；旁聊里没有附件区，字和照片直接发（ADR-0093）。
///
/// 布局照 ADR-0109 第 6 节：Bot 有命令时最左边是“菜单”，然后回形针、输入框，右边那一格（`ComposerSendSlot`）
/// 有字或有附件才出现发送键。
struct ComposerView: View {
    let model: ChatModel
    let composer: ComposerModel
    let pin: BottomPin
    /// 房间里各 Bot 的斜杠命令（ADR-0054）；空的时候没有“菜单”按钮，敲“/”也不弹列表。
    var commands: [RoomCommand] = []
    var commandsOmitted = 0
    /// 命令列表里每条命令旁的 Bot 头像。
    var commandAvatar: (RoomCommand) -> AnyView = { _ in AnyView(EmptyView()) }
    /// 点回复条里的引用，跳到原消息（ADR-0109 第 4 节）。
    var jumpToReply: (String) -> Void = { _ in }
    private var roomId: String { composer.roomId }
    /// 附件区只有主对话有（ADR-0093）：旁聊里字和照片直接发，不给选文件，粘贴文件提示去主对话发。
    var isSideChat: Bool { composer.threadRoot != nil }
    var draftKey: String { model.draftKey(composer.target) }
    var draft: Binding<String> {
        Binding(
            get: { model.drafts[draftKey, default: ""] },
            set: {
                model.drafts[draftKey] = $0
                model.saveDraft(composer.target)
            }
        )
    }
    @State private var photos: [PhotosPickerItem] = []
    @State private var isChoosingAttachment = false
    @State private var isPickingPhoto = false
    @State private var isPickingFiles = false
    /// 附件面板收起以后接着开哪个系统选择器：两张系统面板不能叠着开。
    @State private var afterAttach: AfterAttach?
    /// 点“菜单”按钮打开的命令列表；敲“/”弹出的那一张不看它。
    @State private var showsCommands = false
    /// 点了命令以后输入框里的“/…”原样留着；字还是这一句时列表不再按它弹，动一下字就恢复。
    @State private var sentSlash = ""
    @FocusState private var isFocused: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let isEmpty = draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && composer.attachments.isEmpty
        // 输入栏浮在消息上（ADR-0107、ADR-0109 第 8 节）：菜单胶囊、回形针、输入框、发送键各是一块玻璃，
        // 回复条和附件区收在输入框那一块里；命令列表是它们上方的一张玻璃卡片。
        VStack(spacing: WontonTheme.spacing.s4) {
            if !listedCommands.isEmpty {
                CommandMenu(
                    commands: listedCommands, omitted: commandQuery == nil ? commandsOmitted : 0,
                    showsBot: Set(commands.map(\.botUserId)).count > 1, avatar: commandAvatar, choose: run
                )
                .padding(.horizontal, WontonTheme.spacing.chatFloatInset - WontonTheme.spacing.s4)
            }
            AgentStopNotice(state: composer.agentStop)
            HStack(alignment: .bottom, spacing: WontonTheme.spacing.s4) {
                if !commands.isEmpty {
                    ComposerMenuButton(showsTitle: isEmpty) { showsCommands.toggle() }
                }
                ChatGlassButton(icon: .attach, label: String(localized: .Composer.composerAttach)) {
                    // 附件区满了就不开面板，说一声最多几个。
                    if hasRoom() { isChoosingAttachment = true }
                }
                .accessibilityIdentifier("composerAttach")
                // 编辑只改文字，附件区不动（ADR-0109 第 8 节）。
                .disabled(composer.edit != nil)
                .opacity(composer.edit == nil ? 1 : WontonTheme.ratio.secondaryContent)
                field
                // Bot 在跑时停止钮排在发送钮前面；没字时发送钮不出，只剩停止钮。
                if AgentStopButton.isShown(composer.agentStop) {
                    AgentStopButton(state: composer.agentStop, action: composer.stopAgent)
                }
                ComposerSendSlot(canSend: !isEmpty, editing: composer.edit != nil, send: send)
            }
            .chatGlassCluster()
            // 渐隐从输入栏上方一点开始，一直垫到屏幕底（伸出去的部分在屏幕外或键盘后面）。
            .background {
                ChatEdgeFade(edge: .bottom, fade: WontonTheme.spacing.s32)
                    .padding(.top, -WontonTheme.spacing.s24)
                    .padding(.horizontal, -WontonTheme.spacing.s4)
                    .padding(.bottom, -WontonTheme.size.detailIcon)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s4)
        .padding(.bottom, WontonTheme.spacing.s4)
        // 停止钮的状态跟着输入栏订阅：聊天页不用知道它。
        .task(id: draftKey) { await composer.observeAgentStop() }
        // 打了别的字，“菜单”按钮打开的列表就收起；敲“/”弹的那张跟着字过滤。
        .onChange(of: draft.wrappedValue) { _, text in
            if !text.isEmpty { showsCommands = false }
        }
        // 翻时间线时列表收起，把列表让出来（ADR-0109 第 7 节）。
        .onChange(of: pin.isScrolling) { _, scrolling in
            if scrolling { showsCommands = false }
        }
        // 点了回复就把光标放进输入框。
        .onChange(of: model.replyFocusRevision) {
            if model.replies[draftKey] != nil { isFocused = true }
        }
        .onAppear {
            // 外面发来的文字只进主对话（ADR-0093）：旁聊是主人主动开的，不接外面的文字。
            if !isSideChat { model.acceptPendingDraft(in: composer.target) }
        }
        .sheet(isPresented: $isChoosingAttachment, onDismiss: continueAfterAttach) {
            AttachSheet(
                limit: composer.remaining, allowsFiles: !isSideChat,
                pick: { assets in
                    isChoosingAttachment = false
                    Task { await add(assets) }
                },
                chooseFiles: { leaveAttach(for: .files) },
                openSystemPicker: { leaveAttach(for: .photos) }
            )
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
        }
        // 选几张都只进附件区，不发；最多选到附件区放满。0 在系统选择器里是不限，所以至少给 1，满了根本不打开。
        .photosPicker(
            isPresented: $isPickingPhoto, selection: $photos,
            maxSelectionCount: max(composer.remaining, 1), selectionBehavior: .ordered, matching: .images
        )
        .onChange(of: photos) { _, items in
            guard !items.isEmpty else { return }
            photos = []
            // 旁聊里照片不进附件区，转码好直接发（ADR-0093）；输入框里的字当说明一起发，回复对象跟着第一张照片走。
            if isSideChat {
                Task { await sendPhotos(items) }
            } else {
                Task { await composer.addPhotos(items) }
            }
        }
        .fileImporter(isPresented: $isPickingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) {
            switch $0 {
            case let .success(urls):
                // 文件选择器不能限制选几个：超出的不放，附件区满了的提示由核心给。
                composer.addFiles(urls)
            case let .failure(error):
                model.rejection = Problem(
                    title: String(localized: .Composer.composerChooseFailed), detail: error.localizedDescription
                )
            }
        }
    }

    /// 输入框那一块玻璃：回复条、附件区、文字框从上到下。
    private var field: some View {
        let shape = RoundedRectangle(cornerRadius: WontonTheme.size.chatControl / 2, style: .continuous)
        return VStack(spacing: 0) {
            if let edit = composer.edit {
                EditNotice(summary: edit.summary) { model.cancelEdit(in: composer.target) }
                    .padding(.horizontal, WontonTheme.spacing.s12)
                    .padding(.trailing, WontonTheme.spacing.s4)
                    .padding(.top, WontonTheme.spacing.s8)
            } else if let reply = model.replies[draftKey] {
                ReplyQuoteView(
                    quote: reply, core: model.core, ink: Color(colors.textPrimary), accent: Color(colors.accentInk),
                    cancel: { model.cancelReply(in: composer.target) },
                    jump: { jumpToReply(reply.eventId) }
                )
                .padding(.leading, WontonTheme.spacing.s12)
                .padding(.trailing, WontonTheme.spacing.s4)
                .padding(.top, WontonTheme.spacing.s8)
            }
            ComposerTray(composer: composer)
            ComposerField(
                text: draft,
                focused: $isFocused,
                onPasteImage: { providers in Task { await pasteImage(from: providers) } },
                onPasteFiles: { pasteFiles($0) },
                onSend: send
            )
            // 正在输入（ADR-0109 第 6 节）：核心自己节流，这里每个字都照实报。
            .onChange(of: draft.wrappedValue) { _, value in
                model.sendTyping(
                    !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                    in: composer.target
                )
            }
        }
        .clipShape(shape)
        .chatGlass(in: shape)
        .padding(.vertical, (WontonTheme.size.target - WontonTheme.size.chatControl) / 2)
    }

    /// 输入框开头敲了“/”时，“/”后面的字：命令列表按它做前缀过滤。没敲“/”或者已经换行了为空。
    private var commandQuery: String? {
        let text = draft.wrappedValue
        guard text != sentSlash else { return nil }
        guard text.hasPrefix("/"), !text.contains(where: \.isNewline) else { return nil }
        return String(text.dropFirst())
    }

    /// 这会儿列出来的命令：敲了“/”按前缀过滤，点了“菜单”按钮是整张表，都不是就不列。
    private var listedCommands: [RoomCommand] {
        if let query = commandQuery?.lowercased() {
            return commands.filter { $0.name.lowercased().hasPrefix(query) }
        }
        return showsCommands ? commands : []
    }

    /// 点了一条命令：直接发出核心给的那句，不用再按发送（ADR-0109 第 7 节）。输入框里的草稿是主人正在写的，
    /// 发完留着；回复对象跟着命令发出去，发出去才收起。
    private func run(_ command: RoomCommand) {
        pin.pin()
        sentSlash = draft.wrappedValue
        model.send(.timeline(action: .sendText(
            target: composer.target, body: command.text, replyTo: model.replies[draftKey]?.eventId)))
        model.cancelReply(in: composer.target)
        showsCommands = false
    }

    /// 附件区满了就不开选择器，说一声最多几个。
    private func hasRoom() -> Bool {
        guard composer.remaining == 0 else { return true }
        model.rejection = Problem(title: String(localized: .Composer.composerTrayFull(Int(composer.maxAttachments))))
        return false
    }

    private enum AfterAttach { case files, photos }

    private func leaveAttach(for next: AfterAttach) {
        afterAttach = next
        isChoosingAttachment = false
    }

    private func continueAfterAttach() {
        switch afterAttach {
        case .files: isPickingFiles = true
        case .photos: isPickingPhoto = true
        case nil: break
        }
        afterAttach = nil
    }

    /// 附件面板里选好的照片：取原图的字节，和粘贴进来的图走同一条路，进附件区；旁聊里直接发（ADR-0093）。
    private func add(_ assets: [PHAsset]) async {
        for asset in assets {
            guard let (data, type) = await PhotoLibrary.data(asset) else {
                model.rejection = Problem(
                    title: String(localized: .Photo.photoUnreadable),
                    nextStep: String(localized: .Photo.photoTryAnother)
                )
                continue
            }
            if isSideChat {
                await composer.sendImage(data, type, replyTo: model.replies[draftKey]?.eventId)
            } else {
                await composer.addImage(data, type)
            }
        }
    }

    private func send() {
        guard !draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !composer.attachments.isEmpty else { return }
        pin.pin()
        model.sendTyping(false, in: composer.target)
        if let edit = composer.edit {
            // 核心成功后自己把原草稿和回复对象放回去，编辑条跟着从更新里消失。
            model.editMessage(draft.wrappedValue, edit: edit, in: composer.target)
            return
        }
        // 被核心拒收时字、附件和回复对象都留着，改完再发；收下了才清。
        guard composer.send(draft.wrappedValue, replyTo: model.replies[draftKey]?.eventId) else { return }
        draft.wrappedValue = ""
        model.cancelReply(in: composer.target)
    }
}

/// 输入框右边“发送键那一格”：有字或有附件才出现发送键（ADR-0109 第 6 节），空着时不画东西。
/// 单独一个视图：别的按钮要占这一格（比如 Agent 在跑时的“停止”）就换在这里，不动输入框的布局。
struct ComposerSendSlot: View {
    let canSend: Bool
    var editing = false
    let send: () -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            if canSend {
                SendButton(action: send, editing: editing)
                    .transition(reduceMotion ? .opacity : .scale.combined(with: .opacity))
            }
        }
        // 空着时这一格不占宽，输入框一直伸到右边。
        .padding(.leading, canSend ? 0 : -WontonTheme.spacing.s4)
        .animation(.easeOut(duration: WontonTheme.motion.feedback), value: canSend)
    }
}

private struct SendButton: View {
    let action: () -> Void
    var editing = false

    var body: some View {
        ChatGlassButton(
            icon: editing ? .check : .send,
            label: String(localized: editing ? .Composer.composerEditSend : .Composer.composerSend),
            prominent: true, action: action
        )
        .accessibilityIdentifier(editing ? "composerEditSend" : "composerSend")
    }
}

/// Bot 有命令时输入栏最左边的“菜单”胶囊（ADR-0109 第 6 节）：重点色的玻璃，图标加字；输入框里有字时收成只剩图标，
/// 把宽度让给输入框。
private struct ComposerMenuButton: View {
    let showsTitle: Bool
    let action: () -> Void
    @Environment(\.wontonColors) private var colors
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Button(action: action) {
            HStack(spacing: WontonTheme.spacing.s4) {
                WontonIcon.list.view(size: WontonTheme.size.iconSmall)
                if showsTitle {
                    Text(.Composer.composerCommands)
                        .wontonText(WontonTheme.typography.choice)
                        .lineLimit(1)
                        .fixedSize()
                        .transition(.opacity)
                }
            }
            .foregroundStyle(Color(colors.canvas))
            .padding(.horizontal, WontonTheme.spacing.s12)
            .frame(minWidth: WontonTheme.size.chatControl, minHeight: WontonTheme.size.chatControl)
            .chatGlass(in: Capsule(), tint: Color(colors.accentInk), interactive: true)
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(.Composer.composerCommands))
        .accessibilityIdentifier("composerCommands")
        .animation(
            .easeOut(duration: reduceMotion ? WontonTheme.motion.reduced : WontonTheme.motion.state), value: showsTitle
        )
    }
}

/// 编辑条：说明在改哪条、一行摘要，× 放弃并恢复原草稿（ADR-0109 第 8 节）。
private struct EditNotice: View {
    let summary: String
    let cancel: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            Image(systemName: "pencil")
                .foregroundStyle(Color(colors.textSecondary))
            VStack(alignment: .leading, spacing: 0) {
                Text(.Composer.composerEditing)
                    .font(WontonTheme.font.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                Text(summary)
                    .font(WontonTheme.font.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            WontonIconButton(icon: .close, label: String(localized: .Composer.composerEditCancel), action: cancel)
        }
    }
}
