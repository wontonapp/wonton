import QuickLook
import SwiftUI
import WontonClient

/// 输入框上方进行中和要处理的传输，还有下载完的入口。上传完的已经是房间里的一条消息。
struct TransfersStrip: View {
    let roomId: String
    @Environment(FilesModel.self) private var files
    @State private var isShowingDownloaded = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let view = files.viewModel
        let listed = files.core.roomTransfers(roomId: roomId)
        // 不用 `Group`：它没有自己的视图，内容变空时 `onChange` 可能不触发。
        VStack(spacing: 0) {
            if !listed.here.isEmpty || !listed.elsewhere.isEmpty || !view.downloaded.isEmpty {
                VStack(spacing: WontonTheme.spacing.s8) {
                    ForEach(listed.here, id: \.id) { TransferRow(transfer: $0) }
                    ForEach(listed.elsewhere, id: \.transfer.id) {
                        TransferRow(transfer: $0.transfer, elsewhere: $0)
                    }
                    if !view.downloaded.isEmpty {
                        Button("已下载 \(view.downloaded.count) 个") { isShowingDownloaded = true }
                            .wontonText(WontonTheme.typography.caption)
                            .foregroundStyle(Color(colors.textPrimary))
                            .underline()
                            .buttonStyle(.plain)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s8)
                .overlay(alignment: .top) {
                    Rectangle().fill(Color(colors.separator)).frame(height: WontonTheme.size.hairline)
                }
                .wontonSheet(isPresented: $isShowingDownloaded, title: "已下载") { _ in
                    DownloadedList().environment(files)
                }
            }
        }
        .onChange(of: view.downloaded.isEmpty) { _, empty in
            if empty { isShowingDownloaded = false }
        }
    }
}

private struct DownloadedList: View {
    @Environment(FilesModel.self) private var files

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            ForEach(files.viewModel.downloaded, id: \.id) { TransferRow(transfer: $0) }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }
}

private struct TransferRow: View {
    let transfer: Transfer
    var elsewhere: TransferElsewhere?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            (transfer.direction == .upload ? WontonIcon.upload : WontonIcon.download)
                .view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(transfer.name)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textPrimary))
                    .lineLimit(1)
                if let elsewhere {
                    if let room = elsewhere.roomName {
                        small("发到 \(room)")
                    } else {
                        problem("目标房间不在这台服务器的列表里")
                    }
                }
                detail
            }
            Spacer(minLength: 0)
            TransferButtons(transfer: transfer)
        }
    }

    @ViewBuilder private var detail: some View {
        switch transfer.state {
        case .running:
            ProgressView(value: FileSizes.fraction(transfer))
        case .paused:
            if let because = transfer.pausedBecause {
                problem("暂停，已传 \(FileSizes.text(transfer.transferredBytes))：\(because)")
            } else {
                small("等连接恢复后接着传，已传 \(FileSizes.text(transfer.transferredBytes))")
            }
        case let .linkNotSent(_, message):
            problem("已传完，链接没发出：\(message)")
        case let .failed(_, message):
            problem(message)
        case .done:
            EmptyView()
        }
    }

    private func small(_ text: String) -> some View {
        Text(text).wontonText(WontonTheme.typography.caption).foregroundStyle(Color(colors.textSecondary))
    }

    private func problem(_ text: String) -> some View {
        Text(text).wontonText(WontonTheme.typography.caption).foregroundStyle(Color(colors.danger))
    }
}

/// 按核心给的 `actions` 画按钮。
private struct TransferButtons: View {
    let transfer: Transfer
    @Environment(FilesModel.self) private var files
    @State private var isConfirmingDismiss = false
    @State private var opened: URL?
    @State private var sharing: SharedFile?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: 0) {
            ForEach(transfer.actions, id: \.self, content: button)
        }
        .quickLookPreview($opened)
        .sheet(item: $sharing) { ShareSheet(url: $0.url) }
        .wontonDialog(
            "清理 \(transfer.name)",
            isPresented: $isConfirmingDismiss,
            actions: [
                WontonDialogAction(title: "只清记录") {
                    files.send(.dismissTransfer(id: transfer.id, deleteLocalFile: false))
                },
                WontonDialogAction(title: "连手机上的文件一起删", kind: .danger) {
                    files.send(.dismissTransfer(id: transfer.id, deleteLocalFile: true))
                },
                WontonDialogAction(title: "取消")
            ]
        )
    }

    @ViewBuilder private func button(_ action: TransferAction) -> some View {
        switch action {
        case .cancel:
            WontonIconButton(icon: .close, label: "取消") { files.send(.cancelTransfer(id: transfer.id)) }
        case .dismiss:
            WontonIconButton(icon: .close, label: "清除") {
                files.send(.dismissTransfer(id: transfer.id, deleteLocalFile: false))
            }
        case .dismissDownloaded:
            WontonIconButton(icon: .trash, label: "清理") { isConfirmingDismiss = true }
        case .resendLink:
            textButton("重发链接") { files.send(.resendFileLink(id: transfer.id)) }
        case .stopSending:
            textButton("不发了") { files.send(.cancelTransfer(id: transfer.id)) }
        case .open:
            WontonIconButton(icon: .eye, label: "打开") { opened = localFile }
        case .share:
            WontonIconButton(icon: .share, label: "分享") { sharing = localFile.map(SharedFile.init) }
        }
    }

    private func textButton(_ title: String, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textPrimary))
            .underline()
            .buttonStyle(.plain)
            .frame(minHeight: WontonTheme.size.target)
            .padding(.horizontal, WontonTheme.spacing.s4)
    }

    private var localFile: URL? {
        transfer.destination.map { URL(filePath: $0) }
    }
}
