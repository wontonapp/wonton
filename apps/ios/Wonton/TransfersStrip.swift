import QuickLook
import SwiftUI
import WontonClient

/// 输入框上方进行中和要处理的传输，还有下载完的入口。上传完的已经是房间里的一条消息。
struct TransfersStrip: View {
    let roomId: String
    @Environment(FilesModel.self) private var files
    @State private var isShowingDownloaded = false

    var body: some View {
        let view = files.viewModel
        let listed = files.core.roomTransfers(roomId: roomId)
        // 不用 `Group`：它没有自己的视图，内容变空时 `onChange` 可能不触发。
        VStack(spacing: 0) {
            if !listed.here.isEmpty || !listed.elsewhere.isEmpty || !view.downloaded.isEmpty {
                VStack(spacing: 6) {
                    ForEach(listed.here, id: \.id) { TransferRow(transfer: $0) }
                    ForEach(listed.elsewhere, id: \.transfer.id) {
                        TransferRow(transfer: $0.transfer, elsewhere: $0)
                    }
                    if !view.downloaded.isEmpty {
                        Button("已下载 \(view.downloaded.count) 个") { isShowingDownloaded = true }
                            .font(.caption)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                }
                .padding(.horizontal)
                .padding(.vertical, 6)
                .background(.bar)
                .sheet(isPresented: $isShowingDownloaded) { DownloadedList().environment(files) }
            }
        }
        .onChange(of: view.downloaded.isEmpty) { _, empty in
            if empty { isShowingDownloaded = false }
        }
    }
}

private struct DownloadedList: View {
    @Environment(FilesModel.self) private var files
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List(files.viewModel.downloaded, id: \.id) { TransferRow(transfer: $0) }
                .navigationTitle("已下载")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar { Button("完成") { dismiss() } }
        }
    }
}

private struct TransferRow: View {
    let transfer: Transfer
    var elsewhere: TransferElsewhere?

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: transfer.direction == .upload ? "arrow.up.circle" : "arrow.down.circle")
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(transfer.name).font(.caption).lineLimit(1)
                if let elsewhere {
                    if let room = elsewhere.roomName {
                        Text("发到 \(room)").font(.caption2).foregroundStyle(.secondary)
                    } else {
                        Text("目标房间不在这台服务器的列表里").font(.caption2).foregroundStyle(.red)
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
                Text("暂停，已传 \(FileSizes.text(transfer.transferredBytes))：\(because)")
                    .font(.caption2)
                    .foregroundStyle(.red)
            } else {
                Text("等连接恢复后接着传，已传 \(FileSizes.text(transfer.transferredBytes))")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        case let .linkNotSent(_, message):
            Text("已传完，链接没发出：\(message)").font(.caption2).foregroundStyle(.red)
        case let .failed(_, message):
            Text(message).font(.caption2).foregroundStyle(.red)
        case .done:
            EmptyView()
        }
    }
}

/// 按核心给的 `actions` 画按钮。
private struct TransferButtons: View {
    let transfer: Transfer
    @Environment(FilesModel.self) private var files
    @State private var isConfirmingDismiss = false
    @State private var opened: URL?
    @State private var sharing: SharedFile?

    var body: some View {
        HStack(spacing: 12) {
            ForEach(transfer.actions, id: \.self, content: button)
        }
        .buttonStyle(.borderless)
        .quickLookPreview($opened)
        .sheet(item: $sharing) { ShareSheet(url: $0.url) }
        .confirmationDialog("清理 \(transfer.name)", isPresented: $isConfirmingDismiss, titleVisibility: .visible) {
            Button("只清记录") { files.send(.dismissTransfer(id: transfer.id, deleteLocalFile: false)) }
            Button("连手机上的文件一起删", role: .destructive) {
                files.send(.dismissTransfer(id: transfer.id, deleteLocalFile: true))
            }
            Button("取消", role: .cancel) {}
        }
    }

    @ViewBuilder private func button(_ action: TransferAction) -> some View {
        switch action {
        case .cancel:
            icon("xmark.circle.fill", label: "取消") { files.send(.cancelTransfer(id: transfer.id)) }
        case .dismiss:
            icon("xmark.circle.fill", label: "清除") {
                files.send(.dismissTransfer(id: transfer.id, deleteLocalFile: false))
            }
        case .dismissDownloaded:
            icon("trash", label: "清理") { isConfirmingDismiss = true }
        case .resendLink:
            Button("重发链接") { files.send(.resendFileLink(id: transfer.id)) }.font(.caption)
        case .stopSending:
            Button("不发了") { files.send(.cancelTransfer(id: transfer.id)) }.font(.caption)
        case .open:
            icon("eye", label: "打开") { opened = localFile }
        case .share:
            icon("square.and.arrow.up", label: "分享") { sharing = localFile.map(SharedFile.init) }
        }
    }

    private func icon(_ name: String, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { Image(systemName: name).foregroundStyle(.secondary) }
            .accessibilityLabel(label)
    }

    private var localFile: URL? {
        transfer.destination.map { URL(filePath: $0) }
    }
}
