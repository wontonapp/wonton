import Foundation
import Observation
import UniformTypeIdentifiers
import WontonClient

/// 系统分享进来的分享页（ADR-0097）和核心之间的桥：持有核心推来的 ViewModel，转发认领、删项、发送。
@MainActor
@Observable
final class ShareInModel {
    private(set) var viewModel: ShareInViewModel
    private(set) var sending = false
    /// 没配对时主人点了“去配对”：这一次的分享页先收起，配好以后去向变了再出来。
    private(set) var pairingLater: String?
    var problem: Problem?

    let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.shareIn()
    }

    /// 在根视图的 `.task` 里调用，App 在前台时一直订阅。
    func observe() async {
        for await viewModel in core.subscribeShare().updates() {
            self.viewModel = viewModel
        }
    }

    /// 分享页该不该显示：有待发的分享，且不是主人刚点了“去配对”的那一次。
    var showsSheet: Bool {
        guard let share = viewModel.pending else { return false }
        if case .notPaired = share.destinations, pairingLater == share.id { return false }
        return true
    }

    /// 分享扩展写好收件目录后经 `wonton://share/<令牌>` 打开 App；没配对也认领。
    func claim(_ token: String) {
        do {
            try core.claimShare(token: token)
        } catch {
            problem = error.problem
        }
    }

    func pairLater() {
        pairingLater = viewModel.pending?.id
    }

    /// 主人关掉分享页：不发了。正在发时关掉不算，东西已经交给附件区。
    func dismissed() {
        guard !sending, pairingLater != viewModel.pending?.id, viewModel.pending != nil else { return }
        discard(nil)
    }

    func discard(_ itemId: String?) {
        do {
            try core.discardShare(itemId: itemId)
        } catch {
            problem = error.problem
        }
    }

    /// 照片和从相册选的一样压缩、去掉 EXIF（ADR-0005），读不了的整次不发，不悄悄少发一张。成功返回 true，分享页由核心收起。
    func send(to roomId: String, body: String) async -> Bool {
        guard let share = viewModel.pending, !sending else { return false }
        sending = true
        defer { sending = false }
        let core = core
        do {
            try await Task.detached {
                let images = try share.items.compactMap(Self.prepare)
                try core.sendShare(roomId: roomId, body: body, images: images)
            }.value
            return true
        } catch let failure as PhotoUpload.Failure {
            problem = Problem(
                title: String(localized: failure.text),
                nextStep: String(localized: .Photo.photoTryAnother)
            )
        } catch {
            problem = error.problem
        }
        return false
    }

    nonisolated private static func prepare(_ item: ShareInItem) throws -> ShareInImage? {
        guard case .image = item.kind else { return nil }
        let url = URL(filePath: item.path)
        guard let data = try? Data(contentsOf: url) else { throw PhotoUpload.Failure.unreadable }
        let type = item.mimeType.flatMap { UTType(mimeType: $0) }
        return ShareInImage(itemId: item.id, upload: try PhotoUpload.make(data: data, type: type))
    }
}
