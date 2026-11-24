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
    private var isSideChat: Bool { composer.threadRoot != nil }
    private var draftKey: String { model.draftKey(composer.target) }
    private var draft: Binding<String> {
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
    /// 旁聊里输入栏上那句“附件只能在主对话里发”：默认露出来，关掉不再打扰。
    @State private var showsSideChatHint = true
    /// 附件的字节还在取（比如 iCloud 原图）：发送键那一格换成转圈，画面上看得出点到了。
    @State private var isSendingAttachment = false
    @State private var isPickingPhoto = false
    @State private var isPickingFiles = false
    /// 附件面板收起以后接着开哪个系统选择器：两张系统面板不能叠着开。
    @State private var afterAttach: AfterAttach?
    /// 点“菜单”按钮打开的命令列表；敲“/”弹出的那一张不看它。
    @State private var showsCommands = false
    /// 命令收下以后输入框里的“/…”原样留着；字还是这一句时列表不再按它弹，动一下字就恢复。被拒时不动它，列表照旧。
    @State private var sentSlash = ""
    @FocusState private var isFocused: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let isEmpty = draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && composer.attachments.isEmpty
        // 输入栏浮在消息上（ADR-0107、ADR-0109 第 9 节）：菜单胶囊、回形针、输入框、发送键各是一块玻璃，
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
            if isSideChat && showsSideChatHint {
                SideChatAttachHint(
                    openMain: { model.closeSideChat(in: composer.target.roomId, why: "hint") },
                    close: { showsSideChatHint = false }
                )
                .padding(.horizontal, WontonTheme.spacing.chatFloatInset - WontonTheme.spacing.s4)
                .transition(.opacity)
            }
            if let rejection = model.rejection {
                ComposerRejection(problem: rejection) { model.rejection = nil }
                    .padding(.horizontal, WontonTheme.spacing.chatFloatInset - WontonTheme.spacing.s4)
            }
            composerRow(isEmpty: isEmpty)
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
        // 翻时间线时列表收起，把列表让出来（ADR-0109 第 8 节）。
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
                limit: composer.remaining, allowsFiles: !isSideChat, caption: draft,
                send: { assets in
                    isChoosingAttachment = false
                    Task { await sendPicked(assets) }
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

    /// 输入框开头敲了“/”时，“/”后面的字：命令列表按开头的第一个词过滤。没敲“/”或者已经换行了为空。
    /// 只看到第一个词：命令发出去以后在后面接着打参数（“/model gpt”），那句命令本身没改就不弹回来；
    /// 改到命令名才恢复。
    private var commandQuery: String? {
        let text = draft.wrappedValue
        guard text.hasPrefix("/"), !text.contains(where: \.isNewline) else { return nil }
        guard text.prefix { $0 != " " } != sentSlash else { return nil }
        return String(text.dropFirst())
    }

    /// 这会儿列出来的命令：敲了“/”按第一个词做前缀过滤，点了“菜单”按钮是整张表，都不是就不列。
    private var listedCommands: [RoomCommand] {
        if let query = commandQuery?.lowercased() {
            let word = query.prefix { $0 != " " }
            return commands.filter { $0.name.lowercased().hasPrefix(word) }
        }
        return showsCommands ? commands : []
    }

    /// 点了一条命令：直接发出核心给的那句，不用再按发送（ADR-0109 第 8 节）。输入框里的草稿是主人正在写的，
    /// 发完留着；回复对象跟着命令发出去，发出去才收起。
    private func run(_ command: RoomCommand) {
        pin.pin()
        // 被拒（附件区里有传不上去的）时命令没发出去：草稿、回复条和列表都留着让主人再点。
        // 走和发送键同一个口：附件区里有东西时按 Attachment 送，才会被附件区拦下。
        guard composer.send(command.text, replyTo: model.replies[draftKey]?.eventId) else { return }
        // 收下了才记这一句：被拒时草稿没动，先记会把列表压掉。
        // 记第一个词就够：草稿里本来就带着参数送出去的，也能认出“这一条刚发过”。
        sentSlash = String(draft.wrappedValue.prefix { $0 != " " })
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

    /// 附件面板里选好照片点了发送：照片和附言（就是草稿）作为一次发出去（ADR-0109 第 10 节）。主对话里照片先进附件区，
    /// 再和发送键走同一条路（`SendComposer`），附件区里原有的附件也一起走；旁聊没有附件区，照片一张张直接发，附言只当
    /// 第一张试过的照片的说明（ADR-0093）。取字节的这段时间（比如 iCloud 原图下不下来）发送键那一格转圈；发出去了、
    /// 草稿也还是派出时那句才清，被拒或者等的功夫又打了字都留着，回复对象跟着草稿留。
    private func sendPicked(_ assets: [PHAsset]) async {
        pin.pin()
        isSendingAttachment = true
        defer { isSendingAttachment = false }
        let original = draft.wrappedValue
        let caption = original.trimmingCharacters(in: .whitespacesAndNewlines)
        var replyTo = model.replies[draftKey]?.eventId
        var pendingCaption = caption
        var allSent = true
        for asset in assets {
            guard let (data, type) = await PhotoLibrary.data(asset) else {
                model.rejection = Problem(
                    title: String(localized: .Photo.photoUnreadable),
                    nextStep: String(localized: .Photo.photoTryAnother)
                )
                continue
            }
            if isSideChat {
                // 第一张试过的照片带走附言；被拒也不留给后面的（和 Android 一样）。
                let photoCaption = pendingCaption
                pendingCaption = ""
                if await composer.sendImage(data, type, caption: photoCaption, replyTo: replyTo) {
                    replyTo = nil
                } else {
                    allSent = false
                }
            } else {
                await composer.addImage(data, type)
            }
        }
        guard allSent else { return }
        if isSideChat {
            // 附言已经跟着第一张走了，不再单发一条。
            if draft.wrappedValue == original { draft.wrappedValue = "" }
            model.cancelReply(in: composer.target)
            return
        }
        let action = TimelineAction.sendComposer(target: composer.target, body: caption, replyTo: replyTo)
        guard model.send(.timeline(action: action)) else { return }
        if draft.wrappedValue == original { draft.wrappedValue = "" }
        model.cancelReply(in: composer.target)
    }

    /// 粘贴的图转码后进附件区；旁聊里直接发（ADR-0093）。
    private func pasteImage(from providers: [NSItemProvider]) async {
        guard let (data, type) = await ComposerPaste.image(from: providers) else {
            model.rejection = Problem(
                title: String(localized: .Photo.photoUnreadable), nextStep: String(localized: .Photo.photoTryAnother)
            )
            return
        }
        if isSideChat {
            await sendImage(data, type)
        } else {
            await composer.addImage(data, type)
        }
    }

    private func pasteFiles(_ urls: [URL]) {
        // 文件上传只认房间：旁聊里粘贴文件提示去主对话发（ADR-0093）。
        if isSideChat {
            model.rejection = Problem(title: String(localized: .Chat.sideChatNoFiles))
            return
        }
        guard !urls.isEmpty, urls.allSatisfy(\.isFileURL) else {
            model.rejection = Problem(title: String(localized: .Composer.composerChooseFailed))
            return
        }
        composer.addFiles(urls)
    }

    /// 旁聊里发一张照片，输入框里的字当说明一起发；发出去了才清输入框（转码失败时字留着）。
    private func sendImage(_ data: Data, _ type: UTType?) async {
        let caption = draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard await composer.sendImage(data, type, caption: caption, replyTo: model.replies[draftKey]?.eventId)
        else { return }
        draft.wrappedValue = ""
    }

    /// 旁聊里一次发几张：说明跟第一张走，发出去了才清输入框。
    private func sendPhotos(_ items: [PhotosPickerItem]) async {
        let sent = await composer.sendPhotos(
            items, caption: draft.wrappedValue, replyTo: model.replies[draftKey]?.eventId
        )
        if sent { draft.wrappedValue = "" }
    }

    private func send() {
        guard !draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !composer.attachments.isEmpty else { return }
        pin.pin()
        // 旁聊没有附件区，附件区空着时 `send` 走的就是直接发字。被核心拒收时字、附件和回复对象都留着，改完再发。
        guard composer.send(draft.wrappedValue, replyTo: model.replies[draftKey]?.eventId) else { return }
        draft.wrappedValue = ""
        model.cancelReply(in: composer.target)
    }
}

private extension ComposerView {
    /// 输入栏那一排：菜单、回形针、输入框、发送键各是一块玻璃，靠 `chatGlassCluster` 拼成一整块。
    private func composerRow(isEmpty: Bool) -> some View {
        HStack(alignment: .bottom, spacing: WontonTheme.spacing.s4) {
            if !commands.isEmpty {
                ComposerMenuButton(showsTitle: isEmpty) { showsCommands.toggle() }
            }
            ChatGlassButton(icon: .attach, label: String(localized: .Composer.composerAttach)) {
                // 旁聊没有附件区，点了把那句话露出来；主对话里附件区满了就不开面板，说一声最多几个。
                if isSideChat { showsSideChatHint = true } else if hasRoom() { isChoosingAttachment = true }
            }
            .accessibilityIdentifier("composerAttach")
            field
            // Bot 在跑时停止钮排在发送钮前面；没字时发送钮不出，只剩停止钮。
            if AgentStopButton.isShown(composer.agentStop) {
                AgentStopButton(state: composer.agentStop, action: composer.stopAgent)
            }
            ComposerSendSlot(canSend: !isEmpty, isSending: isSendingAttachment, send: send)
        }
        .chatGlassCluster()
    }

    /// 输入框那一块玻璃：回复条、附件区、文字框从上到下。
    private var field: some View {
        let shape = RoundedRectangle(cornerRadius: WontonTheme.size.chatControl / 2, style: .continuous)
        return VStack(spacing: 0) {
            if let reply = model.replies[draftKey] {
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
        }
        .clipShape(shape)
        .chatGlass(in: shape)
        .padding(.vertical, (WontonTheme.size.target - WontonTheme.size.chatControl) / 2)
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
