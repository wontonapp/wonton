import PhotosUI
import SwiftUI
import WontonClient

/// 聊天页底部的输入栏。
struct ComposerView: View {
    let model: ChatModel
    let roomId: String
    let pin: BottomPin
    private var draft: Binding<String> {
        Binding(get: { model.drafts[roomId, default: ""] }, set: { model.drafts[roomId] = $0 })
    }
    @State private var photo: PhotosPickerItem?
    @State private var isChoosingAttachment = false
    @State private var isPickingPhoto = false
    @State private var isPickingFiles = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let isEmpty = draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        HStack(alignment: .bottom, spacing: 0) {
            WontonIconButton(icon: .attach, label: "附件") { isChoosingAttachment = true }
            ComposerField(text: draft)
                .padding(.vertical, (WontonTheme.size.target - WontonTheme.size.composerField) / 2)
                .padding(.trailing, isEmpty ? WontonTheme.spacing.s12 : 0)
            if !isEmpty {
                SendButton {
                    pin.pin()
                    model.send(.sendText(roomId: roomId, body: draft.wrappedValue))
                    draft.wrappedValue = ""
                }
                .transition(reduceMotion ? .opacity : .scale.combined(with: .opacity))
            }
        }
        .padding(WontonTheme.spacing.s4)
        .animation(.easeOut(duration: WontonTheme.motion.feedback), value: isEmpty)
        .onAppear { model.acceptPendingDraft(in: roomId) }
        .wontonActionSheet(isPresented: $isChoosingAttachment, actions: [
            WontonSheetAction(title: "照片", icon: .image) { isPickingPhoto = true },
            WontonSheetAction(title: "文件", icon: .attach) { isPickingFiles = true }
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
                model.rejection = Problem(title: "选不了这个文件", detail: error.localizedDescription)
            }
        }
    }

    /// 解码和转码放到后台：大照片在主线程上转码会卡住界面。
    private func send(_ item: PhotosPickerItem) async {
        guard let data = try? await item.loadTransferable(type: Data.self) else {
            model.rejection = Problem(title: PhotoUpload.Failure.unreadable.description, nextStep: "换一张试试。")
            return
        }
        let type = item.supportedContentTypes.first
        do {
            let upload = try await Task.detached(priority: .userInitiated) {
                try PhotoUpload.make(data: data, type: type)
            }.value
            pin.pin()
            model.send(.sendImage(roomId: roomId, image: upload))
        } catch {
            model.rejection = Problem(title: String(describing: error), nextStep: "换一张试试。")
        }
    }
}

private struct ComposerField: View {
    @Binding var text: String
    @Environment(\.wontonColors) private var colors
    @FocusState private var isFocused: Bool

    var body: some View {
        TextField("", text: $text, prompt: Text("消息").foregroundStyle(Color(colors.textSecondary)), axis: .vertical)
            .lineLimit(1...6)
            .textFieldStyle(.plain)
            .focused($isFocused)
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(Color(colors.textPrimary))
            .padding(.horizontal, WontonTheme.spacing.s16)
            .padding(.vertical, WontonTheme.spacing.s8)
            .frame(minHeight: WontonTheme.size.composerField)
            .background(
                RoundedRectangle(cornerRadius: WontonTheme.radii.composerField).fill(Color(colors.surfaceSubtle))
            )
            // 系统的输入框只有文字那一行接得住点按，整个底都要能点。
            .contentShape(Rectangle())
            .onTapGesture { isFocused = true }
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
        .accessibilityLabel("发送")
    }
}
