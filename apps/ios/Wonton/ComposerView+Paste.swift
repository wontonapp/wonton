import Photos
import PhotosUI
import SwiftUI

extension ComposerView {
    /// 粘贴的图转码后进附件区；旁聊里直接发（ADR-0093）。
    func pasteImage(from providers: [NSItemProvider]) async {
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

    func pasteFiles(_ urls: [URL]) {
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
    func sendImage(_ data: Data, _ type: UTType?) async {
        let caption = draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard await composer.sendImage(data, type, caption: caption, replyTo: model.replies[draftKey]?.eventId)
        else { return }
        draft.wrappedValue = ""
    }

    /// 旁聊里一次发几张：说明跟第一张走，发出去了才清输入框。
    func sendPhotos(_ items: [PhotosPickerItem]) async {
        let sent = await composer.sendPhotos(
            items, caption: draft.wrappedValue, replyTo: model.replies[draftKey]?.eventId
        )
        if sent { draft.wrappedValue = "" }
    }
}
