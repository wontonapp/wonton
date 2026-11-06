import SwiftUI
import WontonClient

/// 文字消息：排好版的文字，下面是文件卡片。消息只有链接时没有文字。
struct TextMessageBody: View {
    let blocks: [RichBlock]
    let ink: Color
    let files: [FileId]
    var trailingReserve: String?

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            if !blocks.isEmpty { RichTextView(blocks: blocks, ink: ink, trailingReserve: trailingReserve) }
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
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Group {
            switch card {
            case .loading:
                ProgressView().frame(width: WontonTheme.size.fileThumbnail, height: WontonTheme.size.fileThumbnail)
            case let .failed(_, message):
                Button { loads += 1 } label: {
                    Label {
                        Text(message)
                    } icon: {
                        WontonIcon.warning.view(size: WontonTheme.size.iconSmall)
                    }
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.danger))
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
    let info: FileInfo
    let thumbnail: UIImage?
    @Environment(FilesModel.self) private var files
    @Environment(RoomPresenter.self) private var presenter
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let actions = files.actions(for: info)
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Button { presenter.perform(actions.tap, on: info, after: .preview) } label: { summary(actions) }
                .buttonStyle(.plain)
            if let elsewhere = actions.openElsewhere {
                Button(.fileOpenElsewhere) { presenter.perform(elsewhere, on: info, after: .openElsewhere) }
                    .wontonText(WontonTheme.typography.supporting)
                    .underline()
                    .buttonStyle(.plain)
                    .disabled(elsewhere == .wait)
            }
        }
    }

    private func summary(_ actions: FileCardActions) -> some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            icon(playable: actions.tap == .play)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(info.name).wontonText(WontonTheme.typography.supporting).lineLimit(2)
                if let size = info.size { small(FileSizes.text(size)) }
                if let source = sourceText { small(source) }
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
                .frame(width: WontonTheme.size.fileThumbnail, height: WontonTheme.size.fileThumbnail)
                .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
        } else {
            (playable ? WontonIcon.play : WontonIcon.file).view()
                .frame(width: WontonTheme.size.fileThumbnail, height: WontonTheme.size.fileThumbnail)
        }
    }

    @ViewBuilder private func progress(_ download: Transfer?) -> some View {
        switch download?.state {
        case .running:
            if let download { ProgressView(value: FileSizes.fraction(download)) }
        case .paused:
            if let because = download?.pausedBecause {
                problem(String(localized: .filePaused(because)))
            } else {
                small(String(localized: .filePausedWaiting))
            }
        case let .failed(_, message):
            problem(message)
        case .done, .linkNotSent, nil:
            EmptyView()
        }
    }

    private func small(_ text: String) -> some View {
        Text(text).wontonText(WontonTheme.typography.caption).opacity(WontonTheme.ratio.secondaryContent)
    }

    private func problem(_ text: String) -> some View {
        Text(text).wontonText(WontonTheme.typography.caption).foregroundStyle(Color(colors.danger))
    }

    private var sourceText: String? {
        switch info.source {
        case .device: nil
        case let .bot(name, plugin): String(localized: .fileSourceBot(name, plugin))
        case .unknown: String(localized: .fileSourceUnknown)
        }
    }
}

enum FileSizes {
    static func text(_ bytes: UInt64) -> String {
        Int64(clamping: bytes).formatted(.byteCount(style: .file))
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
