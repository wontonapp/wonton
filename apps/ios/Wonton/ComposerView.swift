import PhotosUI
import SwiftUI
import WontonClient

/// 聊天页底部的输入栏。主对话里相册、文件和粘贴进来的东西都先进附件区（ADR-0094），只有发送按钮把字和附件
/// 一起发出去；旁聊里没有附件区，字和照片直接发（ADR-0093）。
struct ComposerView: View {
    let model: ChatModel
    let composer: ComposerModel
    let pin: BottomPin
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
    @State private var isPickingPhoto = false
    @State private var isPickingFiles = false
    @FocusState private var isFocused: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let isEmpty = draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && composer.attachments.isEmpty
        VStack(spacing: 0) {
            if let reply = model.replies[draftKey] {
                ReplyQuoteView(
                    quote: reply, core: model.core, ink: Color(colors.textSecondary),
                    cancel: { model.cancelReply(in: composer.target) }
                )
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.top, WontonTheme.spacing.s8)
            }
            ComposerTray(composer: composer)
            HStack(alignment: .bottom, spacing: 0) {
                WontonIconButton(icon: .attach, label: String(localized: .Composer.composerAttach)) {
                    isChoosingAttachment = true
                }
                ComposerField(
                    text: draft,
                    focused: $isFocused,
                    onPasteImage: { providers in Task { await pasteImage(from: providers) } },
                    onPasteFiles: { pasteFiles($0) },
                    onSend: send
                )
                    .padding(.vertical, (WontonTheme.size.target - WontonTheme.size.composerField) / 2)
                SendButton(action: send)
                    .disabled(isEmpty)
                    .opacity(isEmpty ? WontonTheme.ratio.secondaryContent : 1)
            }
            .padding(WontonTheme.spacing.s4)
        }
        // 点了回复就把光标放进输入框。
        .onChange(of: model.replyFocusRevision) {
            if model.replies[draftKey] != nil { isFocused = true }
        }
        .onAppear {
            // 外面发来的文字只进主对话（ADR-0093）：旁聊是主人主动开的，不接外面的文字。
            if !isSideChat { model.acceptPendingDraft(in: composer.target) }
        }
        .wontonActionSheet(isPresented: $isChoosingAttachment, actions: attachActions())
        // 选几张都只进附件区，不发；最多选到附件区放满。0 在系统选择器里是不限，所以至少给 1，满了根本不打开。
        .photosPicker(
            isPresented: $isPickingPhoto, selection: $photos,
            maxSelectionCount: max(composer.remaining, 1), selectionBehavior: .ordered, matching: .images
        )
        .onChange(of: photos) { _, items in
            guard !items.isEmpty else { return }
            photos = []
            // 旁聊里照片不进附件区，转码好直接发（ADR-0093）；回复对象跟着第一张发出去的照片走。
            if isSideChat {
                Task { await composer.sendPhotos(items, replyTo: model.replies[draftKey]?.eventId) }
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

    /// 附件区满了就不开选择器，说一声最多几个。
    private func hasRoom() -> Bool {
        guard composer.remaining == 0 else { return true }
        model.rejection = Problem(title: String(localized: .Composer.composerTrayFull(Int(composer.maxAttachments))))
        return false
    }

    /// 附件菜单：旁聊里只有图片，发不了文件（ADR-0093）。
    private func attachActions() -> [WontonSheetAction] {
        var actions = [
            WontonSheetAction(title: String(localized: .Composer.composerPhoto), icon: .image) {
                if hasRoom() { isPickingPhoto = true }
            }
        ]
        if !isSideChat {
            actions.append(WontonSheetAction(title: String(localized: .Composer.composerFile), icon: .attach) {
                if hasRoom() { isPickingFiles = true }
            })
        }
        return actions
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
            await composer.sendImage(data, type, replyTo: model.replies[draftKey]?.eventId)
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

    private func send() {
        guard !draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !composer.attachments.isEmpty else { return }
        pin.pin()
        composer.send(draft.wrappedValue, replyTo: model.replies[draftKey]?.eventId)
        draft.wrappedValue = ""
        model.cancelReply(in: composer.target)
    }
}

private struct SendButton: View {
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            WontonIcon.send.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.onPrimary))
                .frame(width: WontonTheme.size.sendButton, height: WontonTheme.size.sendButton)
                .background(Circle().fill(Color(colors.primary)))
                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(.Composer.composerSend))
    }
}
