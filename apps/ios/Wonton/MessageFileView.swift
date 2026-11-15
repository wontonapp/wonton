import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// 文件消息（m.file）的卡片：图标、文件名、大小。点开下载后用 QuickLook 预览，"用其他 App 打开"走分享面板；
/// 下载和打开失败按真实原因显示，点按重试（ADR-0020：内容不可信，下载后交给系统，不在 App 里渲染）。
/// 说明文字由气泡画（和图片一样要气泡的墨色），这里只有卡片本体。
struct MessageFileBody: View {
    let file: FileAttachment
    let core: ClientCore
    @Environment(RoomPresenter.self) private var presenter
    @Environment(\.wontonColors) private var colors
    @State private var loading = false
    @State private var local: URL?
    @State private var problem: String?

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Button { open() } label: {
                HStack(spacing: WontonTheme.spacing.s12) {
                    glyph
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(file.filename).wontonText(WontonTheme.typography.supporting).lineLimit(2)
                        if let size = file.size {
                            small(FileSizes.text(size))
                        }
                        if let problem {
                            Text(problem)
                                .wontonText(WontonTheme.typography.caption)
                                .foregroundStyle(Color(colors.danger))
                        }
                    }
                    .frame(maxWidth: 200, alignment: .leading)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(loading)
            if let local {
                Button(.File.fileOpenElsewhere) {
                    share()
                }
                .wontonText(WontonTheme.typography.supporting)
                .underline()
                .buttonStyle(.plain)
            }
        }
    }

    private var glyph: some View {
        WontonIcon.file.view()
            .frame(width: WontonTheme.size.fileThumbnail, height: WontonTheme.size.fileThumbnail)
            .overlay {
                if loading { ProgressView() }
            }
    }

    private func small(_ text: String) -> some View {
        Text(text).wontonText(WontonTheme.typography.caption).opacity(WontonTheme.ratio.secondaryContent)
    }

    /// 已下载的再点直接预览；没有就先下载。临时文件留在会话里给预览和分享用，系统之后自己清；
    /// 文件没了（系统清了临时目录）就当作没下载过，重新下。
    private func open() {
        if let local, !FileManager.default.fileExists(atPath: local.path) {
            self.local = nil
        }
        if let local {
            presenter.preview = local
            return
        }
        download { url in presenter.preview = url }
    }

    /// 分享已下载的文件；缓存被系统清了就重新下载后接着分享。
    private func share() {
        if let local, !FileManager.default.fileExists(atPath: local.path) {
            self.local = nil
        }
        if let local {
            presenter.sharing = SharedFile(url: local)
            return
        }
        download { url in presenter.sharing = SharedFile(url: url) }
    }

    /// 下载成功后走 then（预览或分享），失败按真实原因提示。
    private func download(then handle: @escaping (URL) -> Void) {
        guard !loading else { return }
        loading = true
        problem = nil
        Task {
            do {
                let data = try await core.loadMedia(media: file.media)
                let url = try Self.temporaryFileURL(filename: file.filename, mimeType: file.mimeType, token: file.media.token, data: data)
                local = url
                handle(url)
            } catch {
                problem = error.problem.title
            }
            loading = false
        }
    }

    /// 文件字节写进临时文件再交给 QuickLook / 分享面板。名字只留最后一段（发送端给的文件名不该带路径），
    /// 没有扩展名时按 MIME 补，QuickLook 和别的 App 才认得类型。
    private static func temporaryFileURL(filename: String, mimeType: String?, token: String, data: Data) throws -> URL {
        var name = (filename as NSString).lastPathComponent
        if name.isEmpty || name == "." || name == ".." { name = "file" }
        if (name as NSString).pathExtension.isEmpty, let mime = mimeType,
           let type = UTType(mimeType: mime), let ext = type.preferredFilenameExtension {
            name += "." + ext
        }
        // token 对同一媒体唯一，放各自的子目录里：不同消息同名文件不互相覆盖，也不会打开到别的消息的内容。
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("files", isDirectory: true)
            .appendingPathComponent(token, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent(name)
        try data.write(to: url, options: .atomic)
        return url
    }
}
