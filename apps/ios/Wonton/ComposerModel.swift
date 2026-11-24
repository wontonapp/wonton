import Foundation
import Observation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// 一个时间线输入框的附件区和点了发送、还没交出去的消息（ADR-0094）。附件进附件区就开始上传，只有发送按钮才发。
/// 附件区只有主对话有（ADR-0093）：旁聊里字和照片直接发，不订阅附件区。
@MainActor
@Observable
final class ComposerModel {
    private(set) var viewModel: ComposerViewModel?
    /// 这条时间线的停止控件（ADR-0061 第 10 节）：核心算好的样子，壳只照着画。
    private(set) var agentStop: RoomAgentStop = .idle
    let roomId: String
    /// 旁聊根的事件 ID；空是房间的主时间线（ADR-0093）。
    let threadRoot: String?
    @ObservationIgnored private let chat: ChatModel

    var core: ClientCore { chat.core }
    var target: TimelineTarget { TimelineTarget(roomId: roomId, threadRoot: threadRoot) }
    var attachments: [ComposerAttachment] { viewModel?.attachments ?? [] }
    var outgoing: [OutgoingMessage] { viewModel?.outgoing ?? [] }
    var maxAttachments: UInt32 { viewModel?.maxAttachments ?? 0 }
    /// 主对话的附件区还剩几个位置；旁聊里照片不停在附件区，选择器读的是 [`photoRoom`]。
    var remaining: Int { max(0, Int(maxAttachments) - attachments.count) }

    /// 一次最多带几张照片：主对话问核心的附件区，旁聊不上附件区、照片选好就发，按核心的默认上限。
    var photoLimit: Int { threadRoot == nil ? Int(maxAttachments) : Self.sideChatPhotos }

    /// 选择器这会儿还能选几张：主对话是附件区余量，旁聊是一次能带的张数。
    var photoRoom: Int { threadRoot == nil ? remaining : photoLimit }

    /// 核心的 `max_attachments` 默认值（`wonton-client` 的 `composer/state.rs` 的 `MAX_ATTACHMENTS`）。
    private static let sideChatPhotos = 10

    init(chat: ChatModel, roomId: String, threadRoot: String? = nil) {
        self.chat = chat
        self.roomId = roomId
        self.threadRoot = threadRoot
    }

    /// 在视图的 `.task` 里调用；视图消失时订阅随之结束。
    func observe() async {
        guard threadRoot == nil else { return }
        let subscription: ComposerSubscription
        do {
            subscription = try core.subscribeComposer(target: target)
        } catch {
            chat.rejection = error.problem
            return
        }
        for await update in subscription.updates() {
            viewModel = update
        }
    }

    /// 在视图的 `.task` 里调用；主对话和旁聊都有。
    func observeAgentStop() async {
        for await update in core.subscribeAgentStop(target: target).updates() {
            agentStop = update
        }
    }

    /// 停下这条时间线里 Bot 正在跑的轮次；结果从 `agentStop` 回来。
    func stopAgent() {
        chat.send(.agent(action: .stopTurns(target: target)))
    }

    /// 相册里选的几张：按选的顺序转码、放进附件区。读不了、转不了的那张报错，别的照样放。
    func addPhotos(_ items: [PhotosPickerItem]) async {
        for item in items {
            guard let data = try? await item.loadTransferable(type: Data.self) else {
                reject(Problem(
                    title: String(localized: .Photo.photoUnreadable),
                    nextStep: String(localized: .Photo.photoTryAnother)
                ))
                continue
            }
            await addImage(data, item.supportedContentTypes.first)
        }
    }

    /// 解码和转码放到后台：大照片在主线程上转码会卡住界面。失败时弹原因，返回空。
    private func transcode(_ data: Data, _ type: UTType?) async -> ImageUpload? {
        do {
            return try await Task.detached(priority: .userInitiated) {
                try PhotoUpload.make(data: data, type: type)
            }.value
        } catch let failure as PhotoUpload.Failure {
            reject(Problem(title: String(localized: failure.text), nextStep: String(localized: .Photo.photoTryAnother)))
            return nil
        } catch {
            reject(error.problem)
            return nil
        }
    }

    /// 解码和转码放到后台：大照片在主线程上转码会卡住界面。
    func addImage(_ data: Data, _ type: UTType?) async {
        guard let upload = await transcode(data, type) else { return }
        let core = core
        let target = target
        do {
            // 核心把字节写进本机的附件区目录，也放到后台。
            let id = try await Task.detached(priority: .userInitiated) {
                try core.addImageAttachment(target: target, image: upload)
            }.value
            failForTestIfAsked(id)
        } catch {
            reject(error.problem)
        }
    }

    /// UI 测试的假数据入口没有真上传：要测“发送被拒”时，让放进来的附件立刻变成上传失败。
    /// 整个进程跑完都算数：开着以后每一张新放进来的附件都会立刻失败，不只是第一张。
    private func failForTestIfAsked(_ attachmentId: String) {
        #if DEBUG
        guard ProcessInfo.processInfo.environment["WONTON_UI_TEST_FAIL_ATTACHMENT"] != nil else { return }
        do {
            try core.failAttachmentForTest(target: target, attachmentId: attachmentId)
        } catch {
            reject(error.problem)
        }
        #endif
    }

    /// 旁聊里粘贴的图：转码好直接发进那个旁聊，不进附件区（附件区只有主对话有，ADR-0093）。
    /// 输入框里的字当图片的说明一起发，回复对象跟着这张照片发出去。发出去返回真；转码失败、核心拒绝都弹原因返回假，回复对象留着。
    @discardableResult
    func sendImage(_ data: Data, _ type: UTType?, caption: String, replyTo: String?) async -> Bool {
        guard let upload = await transcode(data, type) else { return false }
        let caption = caption.trimmingCharacters(in: .whitespacesAndNewlines)
        guard chat.send(.timeline(action: .sendImage(
            target: target, image: upload, replyTo: replyTo, caption: caption.isEmpty ? nil : caption
        ))) else {
            return false
        }
        if replyTo != nil { chat.cancelReply(in: target) }
        return true
    }

    /// 旁聊里选的照片：转码好直接发进那个旁聊，不进附件区（附件区只有主对话有，ADR-0093）。
    /// 说明、回复对象都跟着第一张发出去的照片走；转码失败的照片跳过，原因弹给主人。
    func sendPhotos(_ items: [PhotosPickerItem], caption: String, replyTo: String?) async -> Bool {
        var pendingReply = replyTo
        var pendingCaption = caption
        var sent = false
        for item in items {
            guard let data = try? await item.loadTransferable(type: Data.self) else {
                reject(Problem(
                    title: String(localized: .Photo.photoUnreadable),
                    nextStep: String(localized: .Photo.photoTryAnother)
                ))
                continue
            }
            if await sendImage(data, item.supportedContentTypes.first, caption: pendingCaption, replyTo: pendingReply) {
                pendingReply = nil
                pendingCaption = ""
                sent = true
            }
        }
        return sent
    }

    /// 文件选择器或剪贴板给的文件：按书签记下，App 重启后还能接着传。
    func addFiles(_ urls: [URL]) {
        for url in urls {
            do {
                _ = try core.addFileAttachment(
                    target: target,
                    source: .opaque(handle: try BookmarkFileOpener.handle(for: url)),
                    name: url.lastPathComponent,
                    mimeType: UTType(filenameExtension: url.pathExtension)?.preferredMIMEType
                )
            } catch let error as CoreError {
                reject(error.problem)
            } catch {
                reject(Problem(title: String(localized: .File.fileOpenFailed(error.localizedDescription))))
            }
        }
    }

    /// 输入框的字和附件区作为一次发送。附件区空着、前面也没有待发消息时照旧直接发字：不进待发消息，不闪一下。
    /// 前面有待发消息时字也排在它后面，不抢到前面去。
    /// 被核心收下返回真。被拒（`dispatch` 当场拒绝，比如附件区里有传不上去的）时字、附件和回复对象都留在原地，
    /// 原因由 `chat.rejection` 弹出来；核心收下之后才在队列里失败的照样清，发送队列自己报错。
    @discardableResult
    func send(_ body: String, replyTo: String?) -> Bool {
        if attachments.isEmpty, outgoing.isEmpty {
            return chat.send(.timeline(action: .sendText(target: target, body: body, replyTo: replyTo)))
        }
        return chat.send(.timeline(action: .sendComposer(target: target, body: body, replyTo: replyTo)))
    }

    func remove(_ attachment: ComposerAttachment) {
        chat.send(.timeline(action: .removeAttachment(target: target, attachmentId: attachment.id)))
    }

    func retry(_ attachment: ComposerAttachment) {
        chat.send(.timeline(action: .retryAttachment(target: target, attachmentId: attachment.id)))
    }

    func retry(_ message: OutgoingMessage) {
        chat.send(.timeline(action: .retryOutgoing(target: target, outgoingId: message.id)))
    }

    func cancel(_ message: OutgoingMessage) {
        chat.send(.timeline(action: .cancelOutgoing(target: target, outgoingId: message.id)))
    }

    func thumbnail(_ attachment: ComposerAttachment) async -> UIImage? {
        guard let data = try? await core.attachmentThumbnail(target: target, attachmentId: attachment.id) else {
            return nil
        }
        return UIImage(data: data)
    }

    func reject(_ problem: Problem) {
        chat.rejection = problem
    }
}
