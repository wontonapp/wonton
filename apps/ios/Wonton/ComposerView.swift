import PhotosUI
import SwiftUI
import WontonClient

/// 聊天页底部的输入栏。相册、文件和粘贴进来的东西都先进附件区（ADR-0094），只有发送按钮把字和附件一起发出去。
struct ComposerView: View {
    let model: ChatModel
    let composer: ComposerModel
    let pin: BottomPin
    private var roomId: String { composer.roomId }
    private var draft: Binding<String> {
        Binding(
            get: { model.drafts[roomId, default: ""] },
            set: {
                model.drafts[roomId] = $0
                model.saveDraft(roomId)
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
            if let reply = model.replies[roomId] {
                ReplyQuoteView(
                    quote: reply, core: model.core, ink: Color(colors.textSecondary),
                    cancel: { model.cancelReply(in: roomId) }
                )
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.top, WontonTheme.spacing.s8)
            }
            ComposerTray(composer: composer)
            HStack(alignment: .bottom, spacing: 0) {
                WontonIconButton(icon: .attach, label: String(localized: .composerAttach)) {
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
            if model.replies[roomId] != nil { isFocused = true }
        }
        .onAppear { model.acceptPendingDraft(in: roomId) }
        .wontonActionSheet(isPresented: $isChoosingAttachment, actions: [
            WontonSheetAction(title: String(localized: .composerPhoto), icon: .image) {
                if hasRoom() { isPickingPhoto = true }
            },
            WontonSheetAction(title: String(localized: .composerFile), icon: .attach) {
                if hasRoom() { isPickingFiles = true }
            }
        ])
        // 选几张都只进附件区，不发；最多选到附件区放满。0 在系统选择器里是不限，所以至少给 1，满了根本不打开。
        .photosPicker(
            isPresented: $isPickingPhoto, selection: $photos,
            maxSelectionCount: max(composer.remaining, 1), selectionBehavior: .ordered, matching: .images
        )
        .onChange(of: photos) { _, items in
            guard !items.isEmpty else { return }
            photos = []
            Task { await composer.addPhotos(items) }
        }
        .fileImporter(isPresented: $isPickingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) {
            switch $0 {
            case let .success(urls):
                // 文件选择器不能限制选几个：超出的不放，附件区满了的提示由核心给。
                composer.addFiles(urls)
            case let .failure(error):
                model.rejection = Problem(
                    title: String(localized: .composerChooseFailed), detail: error.localizedDescription
                )
            }
        }
    }

    /// 附件区满了就不开选择器，说一声最多几个。
    private func hasRoom() -> Bool {
        guard composer.remaining == 0 else { return true }
        model.rejection = Problem(title: String(localized: .composerTrayFull(Int(composer.maxAttachments))))
        return false
    }

    /// 粘贴的图转码后进附件区。
    private func pasteImage(from providers: [NSItemProvider]) async {
        guard let (data, type) = await ComposerPaste.image(from: providers) else {
            model.rejection = Problem(
                title: String(localized: .photoUnreadable), nextStep: String(localized: .photoTryAnother)
            )
            return
        }
        await composer.addImage(data, type)
    }

    private func pasteFiles(_ urls: [URL]) {
        guard !urls.isEmpty, urls.allSatisfy(\.isFileURL) else {
            model.rejection = Problem(title: String(localized: .composerChooseFailed))
            return
        }
        composer.addFiles(urls)
    }

    private func send() {
        guard !draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !composer.attachments.isEmpty else { return }
        pin.pin()
        composer.send(draft.wrappedValue, replyTo: model.replies[roomId]?.eventId)
        draft.wrappedValue = ""
        model.cancelReply(in: roomId)
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
        .accessibilityLabel(Text(.composerSend))
    }
}
