import AVKit
import Combine
import SwiftUI
import WontonClient

/// 边下边播。放不了时问核心为什么，说清原因，并给一个下载后用其他 App 打开的按钮。
struct FilePlayer: View {
    let info: FileInfo
    let onOpenElsewhere: () -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(FilesModel.self) private var files
    @State private var player: AVPlayer?
    @State private var problem: FileProblem?
    /// `status` 和通知可能都来：只问一次，第二次问时断线的记录已经被取走，会盖掉正确的原因。
    @State private var isAsking = false

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(info.name)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar { Button(.Common.commonDone) { dismiss() } }
        }
        .task { await start() }
        .onDisappear { player?.pause() }
    }

    @ViewBuilder private var content: some View {
        if let problem {
            ContentUnavailableView {
                Label(.File.filePlaybackUnsupported, systemImage: "exclamationmark.triangle")
            } description: {
                Text(problem.message)
            } actions: {
                Button(.File.fileDownloadAndOpen, action: onOpenElsewhere)
            }
        } else {
            VideoPlayer(player: player)
                .onReceive(failures) { Task { await reportFailure() } }
        }
    }

    /// `AVPlayer` 放不了时不报错，只是不动：开始就放不了看 `status`，放到一半断了看通知。
    private var failures: AnyPublisher<Void, Never> {
        guard let item = player?.currentItem else { return Empty().eraseToAnyPublisher() }
        let failed = item.publisher(for: \.status).filter { $0 == .failed }.map { _ in () }
        let stopped = NotificationCenter.default
            .publisher(for: .AVPlayerItemFailedToPlayToEndTime, object: item)
            .map { _ in () }
        return failed.merge(with: stopped).receive(on: RunLoop.main).eraseToAnyPublisher()
    }

    private func start() async {
        guard let address = try? await files.core.fileStreamUrl(id: info.id), let url = URL(string: address) else {
            await reportFailure()
            return
        }
        // 默认的类别在静音开关打开时没有声音。
        try? AVAudioSession.sharedInstance().setCategory(.playback)
        let player = AVPlayer(url: url)
        self.player = player
        player.play()
    }

    private func reportFailure() async {
        guard !isAsking else { return }
        isAsking = true
        problem = await files.core.filePlaybackProblem(id: info.id)
    }
}
