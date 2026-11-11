import Foundation
import Observation
import UniformTypeIdentifiers
import WontonClient

/// 文件页面和核心之间的桥：传输列表，选好的文件交给核心。被拒绝的操作走聊天页的弹窗。
@MainActor
@Observable
final class FilesModel {
    private(set) var viewModel: TransfersViewModel
    @ObservationIgnored private unowned let chat: ChatModel

    var core: ClientCore { chat.core }

    init(chat: ChatModel) {
        self.chat = chat
        viewModel = chat.core.transfers()
    }

    /// 在视图的 `.task` 里调用；视图消失时 Task 被取消，订阅随之结束。
    func observe() async {
        for await viewModel in core.subscribeTransfers().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        chat.send(action)
    }

    func reject(_ reason: String) {
        chat.rejection = Problem(title: reason)
    }

    /// 传完由核心授权给这个房间，再把链接发进去。
    func upload(_ url: URL, room: String) {
        do {
            send(.uploadFile(
                source: .opaque(handle: try BookmarkFileOpener.handle(for: url)),
                name: url.lastPathComponent,
                mimeType: UTType(filenameExtension: url.pathExtension)?.preferredMIMEType,
                room: room
            ))
        } catch {
            reject(String(localized: .fileOpenFailed(error.localizedDescription)))
        }
    }

    /// 卡片上的按钮。读一下列表：列表变了 SwiftUI 才会重新取。
    func actions(for info: FileInfo) -> FileCardActions {
        _ = viewModel
        return core.fileCardActions(info: info)
    }
}
