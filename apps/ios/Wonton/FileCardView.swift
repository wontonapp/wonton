import QuickLook
import SwiftUI
import WontonClient

/// 文字消息：去掉链接后的文字，下面是文件卡片。消息只有链接时没有文字。
struct TextMessageBody: View {
    let text: String
    let files: [FileId]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !text.isEmpty { Text(text).textSelection(.enabled) }
            ForEach(files, id: \.self) { FileCardView(id: $0) }
        }
    }
}

private struct FileCardView: View {
    let id: FileId
    @Environment(FilesModel.self) private var files
    @State private var card = FileCard.loading
    @State private var thumbnail: UIImage?
    @State private var loads = 0

    var body: some View {
        Group {
            switch card {
            case .loading:
                ProgressView().frame(width: 44, height: 44)
            case let .failed(_, message):
                Button { loads += 1 } label: {
                    Label(message, systemImage: "exclamationmark.triangle").font(.footnote)
                }
                .buttonStyle(.plain)
            case let .ready(info):
                ReadyFileCard(info: info, thumbnail: thumbnail)
            }
        }
        .task(id: loads) { await load() }
    }

    private func load() async {
        card = await files.core.fileCard(id: id)
        guard case let .ready(info) = card, info.hasThumbnail,
              let data = try? await files.core.fileThumbnail(id: id) else { return }
        // 缩略图只是装饰：取不到时画图标，卡片的名字和大小照常显示。
        thumbnail = UIImage(data: data)
    }
}

/// 按核心给的 `FileCardActions` 画按钮。Bot 给的文件内容不可信，下载后交给系统，不在 App 里渲染（ADR-0020）。
private struct ReadyFileCard: View {
    /// 下完之后做什么：用户已经点过了，不用再点一次。
    private enum AfterDownload {
        case preview
        case openElsewhere
    }

    let info: FileInfo
    let thumbnail: UIImage?
    @Environment(FilesModel.self) private var files
    @State private var opened: URL?
    @State private var sharing: SharedFile?
    @State private var isPlaying = false
    @State private var opensElsewhereAfterPlayer = false
    @State private var afterDownload: AfterDownload?

    var body: some View {
        let actions = files.actions(for: info)
        VStack(alignment: .leading, spacing: 4) {
            Button { perform(actions.tap, after: .preview) } label: { summary(actions) }
                .buttonStyle(.plain)
            if let elsewhere = actions.openElsewhere {
                Button("用其他 App 打开") { perform(elsewhere, after: .openElsewhere) }
                    .font(.caption)
                    .disabled(elsewhere == .wait)
            }
        }
        .onChange(of: actions.openElsewhere ?? actions.tap) { _, action in finishDownload(action) }
        .quickLookPreview($opened)
        .sheet(item: $sharing) { ShareSheet(url: $0.url) }
        .fullScreenCover(isPresented: $isPlaying, onDismiss: openElsewhereAfterPlayer) {
            FilePlayer(info: info) {
                opensElsewhereAfterPlayer = true
                isPlaying = false
            }
            .environment(files)
        }
    }

    private func summary(_ actions: FileCardActions) -> some View {
        HStack(spacing: 10) {
            icon(playable: actions.tap == .play)
            VStack(alignment: .leading, spacing: 2) {
                Text(info.name).font(.subheadline).lineLimit(2)
                if let size = info.size { Text(FileSizes.text(size)).font(.caption2).foregroundStyle(.secondary) }
                if let source = sourceText { Text(source).font(.caption2).foregroundStyle(.secondary) }
                progress(actions.download)
            }
            .frame(maxWidth: 200, alignment: .leading)
        }
        .contentShape(Rectangle())
    }

    @ViewBuilder private func icon(playable: Bool) -> some View {
        if let thumbnail {
            Image(uiImage: thumbnail)
                .resizable()
                .scaledToFill()
                .frame(width: 44, height: 44)
                .clipShape(RoundedRectangle(cornerRadius: 6))
        } else {
            Image(systemName: playable ? "play.circle" : "doc")
                .font(.title2)
                .frame(width: 44, height: 44)
        }
    }

    @ViewBuilder private func progress(_ download: Transfer?) -> some View {
        switch download?.state {
        case .running:
            if let download { ProgressView(value: FileSizes.fraction(download)) }
        case .paused:
            if let because = download?.pausedBecause {
                Text("暂停：\(because)").font(.caption2).foregroundStyle(.red)
            } else {
                Text("等连接恢复后接着下载").font(.caption2).foregroundStyle(.secondary)
            }
        case let .failed(_, message):
            Text(message).font(.caption2).foregroundStyle(.red)
        case .done, .linkNotSent, nil:
            EmptyView()
        }
    }

    private func perform(_ action: FileCardAction, after: AfterDownload) {
        switch action {
        case .play:
            isPlaying = true
        case .download:
            afterDownload = after
            files.send(.downloadFile(id: info.id))
        case let .open(path):
            show(path, after)
        case .wait:
            break
        }
    }

    private func finishDownload(_ action: FileCardAction) {
        guard let after = afterDownload else { return }
        switch action {
        case let .open(path):
            afterDownload = nil
            show(path, after)
        case .download:
            afterDownload = nil
        case .play, .wait:
            break
        }
    }

    private func show(_ path: String, _ after: AfterDownload) {
        switch after {
        case .preview: opened = URL(filePath: path)
        case .openElsewhere: sharing = SharedFile(url: URL(filePath: path))
        }
    }

    /// 全屏盖住时弹不出别的，等它关掉。
    private func openElsewhereAfterPlayer() {
        guard opensElsewhereAfterPlayer else { return }
        opensElsewhereAfterPlayer = false
        if let elsewhere = files.actions(for: info).openElsewhere { perform(elsewhere, after: .openElsewhere) }
    }

    private var sourceText: String? {
        switch info.source {
        case .device: nil
        case let .bot(name, plugin): "来自 \(name)（\(plugin)）"
        case .unknown: "来源不明"
        }
    }
}

enum FileSizes {
    static func text(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file)
    }

    /// 还不知道总大小时为空，进度条显示为不确定。
    static func fraction(_ transfer: Transfer) -> Double? {
        guard let total = transfer.totalBytes, total > 0 else { return nil }
        return Double(transfer.transferredBytes) / Double(total)
    }
}

struct SharedFile: Identifiable {
    let url: URL

    var id: URL { url }
}

/// 系统的分享面板：用其他 App 打开、存到“文件”。
struct ShareSheet: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
