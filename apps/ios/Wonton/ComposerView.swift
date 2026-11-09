import PhotosUI
import SwiftUI
import WontonClient

/// 聊天页底部的输入栏。
struct ComposerView: View {
    let model: ChatModel
    let roomId: String
    let pin: BottomPin
    private var draft: Binding<String> {
        Binding(
            get: { model.drafts[roomId, default: ""] },
            set: {
                model.drafts[roomId] = $0
                model.saveDraft(roomId)
            }
        )
    }
    @State private var photo: PhotosPickerItem?
    @State private var isChoosingAttachment = false
    @State private var isPickingPhoto = false
    @State private var isPickingFiles = false
    /// 粘贴进来还没发的图：转码完成待确认。
    @State private var pendingImage: ImageUpload?
    @FocusState private var isFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let isEmpty = draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        VStack(spacing: 0) {
            if let reply = model.replies[roomId] {
                ReplyQuoteView(
                    quote: reply, core: model.core, ink: Color(colors.textSecondary),
                    cancel: { model.cancelReply(in: roomId) }
                )
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.top, WontonTheme.spacing.s8)
            }
            HStack(alignment: .bottom, spacing: 0) {
                WontonIconButton(icon: .attach, label: String(localized: .composerAttach)) { isChoosingAttachment = true }
                ComposerField(
                    text: draft,
                    focused: $isFocused,
                    onPasteImage: { providers in Task { await pasteImage(from: providers) } },
                    onPasteFiles: { pasteFiles($0) }
                )
                    .padding(.vertical, (WontonTheme.size.target - WontonTheme.size.composerField) / 2)
                    .padding(.trailing, isEmpty ? WontonTheme.spacing.s12 : 0)
                if !isEmpty {
                    SendButton {
                        pin.pin()
                        model.send(.sendText(
                            roomId: roomId, body: draft.wrappedValue, replyTo: model.replies[roomId]?.eventId
                        ))
                        draft.wrappedValue = ""
                        model.cancelReply(in: roomId)
                    }
                    .transition(reduceMotion ? .opacity : .scale.combined(with: .opacity))
                }
            }
            .padding(WontonTheme.spacing.s4)
        }
        // 点了回复就把光标放进输入框。
        .onChange(of: model.replies[roomId] != nil) { _, replying in
            if replying { isFocused = true }
        }
        .animation(.easeOut(duration: WontonTheme.motion.feedback), value: isEmpty)
        .onAppear { model.acceptPendingDraft(in: roomId) }
        .wontonActionSheet(isPresented: $isChoosingAttachment, actions: [
            WontonSheetAction(title: String(localized: .composerPhoto), icon: .image) { isPickingPhoto = true },
            WontonSheetAction(title: String(localized: .composerFile), icon: .attach) { isPickingFiles = true }
        ])
        .photosPicker(isPresented: $isPickingPhoto, selection: $photo, matching: .images)
        .onChange(of: photo) { _, item in
            guard let item else { return }
            photo = nil
            Task { await send(item) }
        }
        .fileImporter(isPresented: $isPickingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) {
            switch $0 {
            case let .success(urls):
                pin.pin()
                for url in urls { model.files.upload(url, room: roomId) }
            case let .failure(error):
                model.rejection = Problem(
                    title: String(localized: .composerChooseFailed), detail: error.localizedDescription
                )
            }
        }
        .wontonSheet(isPresented: isPreviewing) { close in
            if let image = pendingImage {
                PastePreview(
                    image: image,
                    onSend: { close { deliver(image) } },
                    onCancel: { close {} }
                )
            }
        }
    }

    /// 预览面板开着表示有待确认的图；往下拖、点空白也走这里取消。
    private var isPreviewing: Binding<Bool> {
        Binding(
            get: { pendingImage != nil },
            set: { if !$0 { pendingImage = nil } }
        )
    }

    /// 相册选的图直接发。
    private func send(_ item: PhotosPickerItem) async {
        guard let data = try? await item.loadTransferable(type: Data.self) else {
            model.rejection = Problem(
                title: String(localized: .photoUnreadable), nextStep: String(localized: .photoTryAnother)
            )
            return
        }
        await transcode(data, item.supportedContentTypes.first, confirm: false)
    }

    /// 粘贴的图先转码，出预览面板等主人确认。
    private func pasteImage(from providers: [NSItemProvider]) async {
        guard let (data, type) = await ComposerPaste.image(from: providers) else {
            model.rejection = Problem(
                title: String(localized: .photoUnreadable), nextStep: String(localized: .photoTryAnother)
            )
            return
        }
        await transcode(data, type, confirm: true)
    }

    /// 粘贴的文件：和文件选择器一样交核心上传，传完链接发进房间。
    private func pasteFiles(_ urls: [URL]) {
        pin.pin()
        for url in urls { model.files.upload(url, room: roomId) }
    }

    /// 解码和转码放到后台：大照片在主线程上转码会卡住界面。`confirm` 为真先出预览再发。
    private func transcode(_ data: Data, _ type: UTType?, confirm: Bool) async {
        do {
            let upload = try await Task.detached(priority: .userInitiated) {
                try PhotoUpload.make(data: data, type: type)
            }.value
            if confirm {
                pendingImage = upload
            } else {
                deliver(upload)
            }
        } catch {
            switch error {
            case let failure as PhotoUpload.Failure:
                model.rejection = Problem(
                    title: String(localized: failure.text), nextStep: String(localized: .photoTryAnother)
                )
            default:
                model.rejection = Problem(
                    title: String(describing: error), nextStep: String(localized: .photoTryAnother)
                )
            }
        }
    }

    private func deliver(_ image: ImageUpload) {
        pendingImage = nil
        pin.pin()
        model.send(.sendImage(roomId: roomId, image: image, replyTo: model.replies[roomId]?.eventId))
        // 回复用掉了：图片不带输入框里的字，正文的草稿留着。
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

/// 粘贴图片的确认面板：看一眼再发，不要就取消。
private struct PastePreview: View {
    let image: ImageUpload
    let onSend: () -> Void
    let onCancel: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s16) {
            // 预览的就是发出去的那份：有缩略图用缩略图，小图直接用原图。
            if let preview = UIImage(data: image.thumbnail?.data ?? image.data) {
                Image(uiImage: preview)
                    .resizable()
                    .scaledToFit()
                    .frame(maxHeight: WontonTheme.size.screenshot)
                    .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
                    .accessibilityHidden(true)
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(action: onCancel) {
                    Text(.commonCancel).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                Button(action: onSend) {
                    Text(.composerSend).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.bottom, WontonTheme.spacing.s8)
    }
}
