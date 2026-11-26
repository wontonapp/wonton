import Observation
import QuickLook
import SwiftUI
import WontonClient

/// 消息打开的页面挂在房间上。列表的行被顶出屏幕就被拆掉，挂在行上的页面跟着关
/// （iOS 26.5 模拟器实测：放视频时来一条长回复，列表跟到底，播放器自己关了）。
@MainActor
@Observable
final class RoomPresenter {
    /// 下完之后做什么：用户已经点过了，不用再点一次。
    enum AfterDownload {
        case preview
        case openElsewhere
    }

    enum CoverContent {
        case player(FileInfo)
        case original(PhotoGroup, startIndex: Int)
    }

    struct Cover: Identifiable {
        let id = UUID()
        var content: CoverContent
    }

    /// 等着下完的那个文件现在的按钮。带上文件：换了一个文件等，不算这个文件的下载结束了。
    struct WaitedAction: Equatable {
        let file: FileId
        let action: FileCardAction
    }

    var cover: Cover?
    var preview: URL?
    var sharing: SharedFile?
    let files: FilesModel
    /// 触发下载时人在的房间：下载完的提示只列在这个房间里，别的房间不显示。
    let roomId: String
    private var waiting: (info: FileInfo, after: AfterDownload)?
    @ObservationIgnored private var elsewhereAfterCover: FileInfo?

    init(files: FilesModel, roomId: String) {
        self.files = files
        self.roomId = roomId
    }

    var waitedAction: WaitedAction? {
        waiting.map { WaitedAction(file: $0.info.id, action: Self.finalAction(files.actions(for: $0.info))) }
    }

    func perform(_ action: FileCardAction, on info: FileInfo, after: AfterDownload) {
        switch action {
        case .play:
            cover = Cover(content: .player(info))
        case .download:
            waiting = (info, after)
            files.send(.files(action: .downloadFile(id: info.id, room: roomId)))
        case let .open(path):
            show(path, after)
        case .wait:
            break
        }
    }

    /// 从点开的那张起翻；单张也带着组键，之后组里补上的照片能跟着出来。
    func showOriginal(_ image: ImageContent, group: PhotoGroup?) {
        let resolved = group ?? PhotoGroup(senderId: "", id: "", images: [image])
        cover = Cover(content: .original(resolved, startIndex: resolved.images.firstIndex(of: image) ?? 0))
    }

    /// 时间线变了：查看器里那组按现有的重取；当前那张没了就停原位置，整组没了就关掉。
    func refreshOriginal(_ groups: [PhotoGroupKey: [ImageContent]]) {
        guard case let .original(group, startIndex) = cover?.content else { return }
        // 单张没有组键，原样留着。
        let images = groups[PhotoGroupKey(senderId: group.senderId, id: group.id)] ?? group.images
        guard !images.isEmpty else {
            cover = nil
            return
        }
        let shown = group.images.indices.contains(startIndex) ? group.images[startIndex] : nil
        let kept = shown.flatMap { images.firstIndex(of: $0) }
        cover?.content = .original(
            PhotoGroup(senderId: group.senderId, id: group.id, images: images),
            startIndex: kept ?? min(startIndex, images.count - 1),
        )
    }

    func closeCoverToOpenElsewhere(_ info: FileInfo) {
        elsewhereAfterCover = info
        cover = nil
    }

    /// 全屏盖住时弹不出别的，等它关掉。
    func coverClosed() {
        guard let info = elsewhereAfterCover else { return }
        elsewhereAfterCover = nil
        if let elsewhere = files.actions(for: info).openElsewhere {
            perform(elsewhere, on: info, after: .openElsewhere)
        }
    }

    func finishDownload(_ action: FileCardAction) {
        guard let after = waiting?.after else { return }
        switch action {
        case let .open(path):
            waiting = nil
            show(path, after)
        case .download:
            waiting = nil
        case .play, .wait:
            break
        }
    }

    private func show(_ path: String, _ after: AfterDownload) {
        switch after {
        case .preview: preview = URL(filePath: path)
        case .openElsewhere: sharing = SharedFile(url: URL(filePath: path))
        }
    }

    private static func finalAction(_ actions: FileCardActions) -> FileCardAction {
        actions.openElsewhere ?? actions.tap
    }
}

struct RoomPresentations: ViewModifier {
    @Bindable var presenter: RoomPresenter

    func body(content: Content) -> some View {
        content
            .onChange(of: presenter.waitedAction) { old, new in
                if let new, old?.file == new.file { presenter.finishDownload(new.action) }
            }
            .quickLookPreview($presenter.preview)
            .sheet(item: $presenter.sharing) { ShareSheet(url: $0.url) }
            .fullScreenCover(item: $presenter.cover, onDismiss: presenter.coverClosed) { cover in
                switch cover.content {
                case let .player(info):
                    FilePlayer(info: info) { presenter.closeCoverToOpenElsewhere(info) }
                        .environment(presenter.files)
                case let .original(group, startIndex):
                    ImageViewer(images: group.images, startIndex: startIndex, core: presenter.files.core)
                }
            }
    }
}
