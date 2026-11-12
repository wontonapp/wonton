import SwiftUI
import UIKit
import AVFoundation
import ImageIO
import WontonClient

/// Bot 的头像（ADR-0044）：有状态头像就播它（`botCurrentAvatar`），否则静态头像，都没有用插件图标。
/// 圆形；读屏读「名字，状态」，状态不确定时只读名字。省电规则在 `AvatarMediaView`。
struct BotAvatarView: View {
    let bot: BotView?
    let pluginIcon: PluginIcon?
    let core: ClientCore
    let model: PluginsModel
    let size: CGFloat
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let current = bot.flatMap(botCurrentAvatar)
        ZStack {
            if let avatar = bot?.avatar {
                AvatarMediaView(image: avatar, core: core, size: size)
            } else {
                PluginIconView(icon: pluginIcon, size: size, model: model)
            }
            if let current {
                AvatarMediaView(image: current, core: core, size: size)
                    .transition(.opacity)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .overlay { Circle().strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline) }
        // 状态头像换挡时新旧淡入淡出；令牌是媒体地址，资料没变不重播。
        .animation(.easeInOut(duration: WontonTheme.motion.state), value: current?.media.token)
        .accessibilityLabel(a11yLabel)
    }

    private var a11yLabel: String {
        guard let bot else { return "" }
        guard let activity = bot.activity else { return bot.name }
        return String(localized: .Chat.botStateLabel(bot.name, activity.text))
    }
}

/// 一个头像媒体：Still 解码、Animated 逐帧、Video 静音循环。省电规则（ADR-0044）：
/// 只在头像在屏幕上且 App 在前台时播；低电量或「减弱动态效果」只显示 poster；同时播放的全局最多 4 个。
struct AvatarMediaView: View {
    let image: AvatarImage
    let core: ClientCore
    let size: CGFloat

    var body: some View {
        switch image.kind {
        case .still: StillAvatarImage(media: image.media, core: core, size: size)
        case .animated: AnimatedAvatarImage(image: image, core: core, size: size)
        case .video: VideoAvatarImage(image: image, core: core, size: size)
        }
    }
}

/// 静止图：字节经 `loadMedia` 解码，按媒体地址放进全局 NSCache（ADR-0044）。
private struct StillAvatarImage: View {
    let media: MediaHandle
    let core: ClientCore
    let size: CGFloat
    @State private var image: UIImage?

    var body: some View {
        ZStack {
            if let image { Image(uiImage: image).resizable().scaledToFill() }
        }
        .frame(width: size, height: size)
        .task(id: media.token) { image = await AvatarImages.decoded(media, from: core) }
    }
}

/// 动图（GIF、动图 WebP）：ImageIO 的逐帧回调循环，取不到文件时由外层落到 poster。
private struct AnimatedAvatarImage: View {
    let image: AvatarImage
    let core: ClientCore
    let size: CGFloat
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isLowPower = ProcessInfo.processInfo.isLowPowerModeEnabled
    @State private var isOnScreen = false
    @State private var playing = false
    @State private var poster: UIImage?
    @State private var firstFrame: UIImage?
    @State private var frame: UIImage?

    private var eligible: Bool {
        isOnScreen && scenePhase == .active && !isLowPower && !reduceMotion
    }

    var body: some View {
        ZStack {
            if playing, let frame {
                Image(uiImage: frame).resizable().scaledToFill()
            } else if let still = poster ?? firstFrame {
                Image(uiImage: still).resizable().scaledToFill()
            }
        }
        .frame(width: size, height: size)
        .animation(.linear(duration: WontonTheme.motion.state), value: playing)
        .task(id: image.media.token) { await loadStills() }
        .task(id: playKey) { await run() }
        .onAppear { isOnScreen = true }
        .onDisappear { isOnScreen = false }
        .onReceive(NotificationCenter.default.publisher(for: .NSProcessInfoPowerStateDidChange)) { _ in
            isLowPower = ProcessInfo.processInfo.isLowPowerModeEnabled
        }
    }

    private var playKey: String { "\(eligible)|\(image.media.token)" }

    private func loadStills() async {
        if let posterMedia = image.poster {
            poster = await AvatarImages.decoded(posterMedia, from: core)
        }
        if firstFrame == nil, poster == nil {
            firstFrame = await AvatarImages.firstFrame(media: image.media, from: core)
        }
    }

    private func run() async {
        guard eligible else {
            playing = false
            return
        }
        // 拿不到名额就停在 poster（ADR-0044：一屏同时最多播 4 个）。
        guard PlaybackBudget.shared.tryAcquire() else {
            playing = false
            return
        }
        defer {
            playing = false
            PlaybackBudget.shared.release()
        }
        guard let path = try? await core.avatarFile(media: image.media) else { return }
        playing = true
        let flag = StopFlag()
        await withTaskCancellationHandler {
            await AnimatedAvatarLoop.run(url: URL(fileURLWithPath: path), flag: flag) { image in
                frame = image
            }
        } onCancel: {
            flag.cancel()
        }
    }
}

/// 视频（MP4，H.264）：AVQueuePlayer 静音循环，AVPlayerLayer 填满圆形。第一帧渲染出来之前，下面垫着 poster。
private struct VideoAvatarImage: View {
    let image: AvatarImage
    let core: ClientCore
    let size: CGFloat
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isLowPower = ProcessInfo.processInfo.isLowPowerModeEnabled
    @State private var isOnScreen = false
    @State private var playing = false
    @State private var poster: UIImage?
    @State private var firstFrame: UIImage?
    @State private var fileURL: URL?

    private var eligible: Bool {
        isOnScreen && scenePhase == .active && !isLowPower && !reduceMotion
    }

    var body: some View {
        ZStack {
            if let still = poster ?? firstFrame {
                Image(uiImage: still).resizable().scaledToFill()
            }
            if playing, let fileURL {
                VideoAvatarLayer(url: fileURL)
            }
        }
        .frame(width: size, height: size)
        .task(id: image.media.token) { await loadStills() }
        .task(id: playKey) { await run() }
        .onAppear { isOnScreen = true }
        .onDisappear { isOnScreen = false }
        .onReceive(NotificationCenter.default.publisher(for: .NSProcessInfoPowerStateDidChange)) { _ in
            isLowPower = ProcessInfo.processInfo.isLowPowerModeEnabled
        }
    }

    private var playKey: String { "\(eligible)|\(image.media.token)" }

    private func loadStills() async {
        if let posterMedia = image.poster {
            poster = await AvatarImages.decoded(posterMedia, from: core)
        }
        if poster == nil, firstFrame == nil {
            firstFrame = await AvatarImages.videoFirstFrame(media: image.media, from: core)
        }
    }

    private func run() async {
        guard eligible else {
            playing = false
            fileURL = nil
            return
        }
        guard PlaybackBudget.shared.tryAcquire() else {
            playing = false
            fileURL = nil
            return
        }
        defer {
            playing = false
            PlaybackBudget.shared.release()
        }
        guard let path = try? await core.avatarFile(media: image.media) else { return }
        fileURL = URL(fileURLWithPath: path)
        playing = true
        // 名额由这个 task 持有，直到视图消失或参数变化（task 被取消）。
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(3600))
        }
    }
}

/// 循环播放的层：静音、填满、随视图销毁释放（dismantle 里停掉）。
private struct VideoAvatarLayer: UIViewRepresentable {
    let url: URL

    final class PlayerView: UIView {
        override static var layerClass: AnyClass { AVPlayerLayer.self }
        var playerLayer: AVPlayerLayer? { layer as? AVPlayerLayer }
    }

    func makeUIView(context: Context) -> PlayerView {
        let view = PlayerView()
        view.playerLayer?.videoGravity = .resizeAspectFill
        return view
    }

    func updateUIView(_ view: PlayerView, context: Context) {
        if context.coordinator.url != url { context.coordinator.start(url, in: view) }
    }

    static func dismantleUIView(_ view: PlayerView, coordinator: Coordinator) {
        coordinator.stop()
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    @MainActor
    final class Coordinator {
        private var player: AVQueuePlayer?
        private var looper: AVPlayerLooper?
        var url: URL?

        func start(_ url: URL, in view: PlayerView) {
            stop()
            self.url = url
            let item = AVPlayerItem(url: url)
            let queue = AVQueuePlayer(items: [item])
            // 不出声也不激活音频会话：静音的 AVPlayer 不碰 AVAudioSession（ADR-0044）。
            queue.isMuted = true
            looper = AVPlayerLooper(player: queue, templateItem: item)
            player = queue
            view.playerLayer?.player = queue
            queue.play()
        }

        func stop() {
            player?.pause()
            looper?.disableLooping()
            looper = nil
            player = nil
            url = nil
        }
    }
}

/// 一屏同时播的视频、动图最多 4 个（ADR-0044），名额全局一份，拿不到就显示 poster。
final class PlaybackBudget: @unchecked Sendable {
    static let shared = PlaybackBudget()

    private let lock = NSLock()
    private var playing = 0

    func tryAcquire() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard playing < 4 else { return false }
        playing += 1
        return true
    }

    func release() {
        lock.lock()
        defer { lock.unlock() }
        playing = max(0, playing - 1)
    }
}

/// 逐帧回调的取消标记：CGAnimate 的循环只能靠块里的 stop 退出。
private final class StopFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false

    var isCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelled
    }

    func cancel() {
        lock.lock()
        cancelled = true
        lock.unlock()
    }
}

private enum AnimatedAvatarLoop {
    /// CGAnimateImageAtURLWithBlock 阻塞调用线程直到 stop，放到后台线程跑；帧跳回主线程。
    static func run(url: URL, flag: StopFlag, frame: @escaping @MainActor @Sendable (UIImage) -> Void) async {
        await Task.detached(priority: .utility) {
            CGAnimateImageAtURLWithBlock(url as CFURL, nil) { _, cgImage, stop in
                if flag.isCancelled {
                    stop.pointee = true
                    return
                }
                let image = UIImage(cgImage: cgImage)
                Task { @MainActor in frame(image) }
            }
        }.value
    }
}

/// 解码好的头像字节缓存，按媒体地址（ADR-0044：同一个媒体只经核心下一次）。
enum AvatarImages {
    // NSCache 本身线程安全，告诉并发检查这一点。
    nonisolated(unsafe) private static let cache = NSCache<NSString, UIImage>()

    static func decoded(_ media: MediaHandle, from core: ClientCore) async -> UIImage? {
        if let cached = cache.object(forKey: media.token as NSString) { return cached }
        guard let data = try? await core.loadMedia(media: media),
              let image = UIImage(data: data) else { return nil }
        cache.setObject(image, forKey: media.token as NSString)
        return image
    }

    /// 动图的第一帧：poster 缺席时垫底。
    static func firstFrame(media: MediaHandle, from core: ClientCore) async -> UIImage? {
        guard let path = try? await core.avatarFile(media: media),
              let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
              let cgImage = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return nil }
        return UIImage(cgImage: cgImage)
    }

    /// 视频的第一帧：poster 缺席时壳自己从视频取（ADR-0044）。
    static func videoFirstFrame(media: MediaHandle, from core: ClientCore) async -> UIImage? {
        guard let path = try? await core.avatarFile(media: media) else { return nil }
        let asset = AVURLAsset(url: URL(fileURLWithPath: path))
        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        return await withCheckedContinuation { continuation in
            generator.generateCGImagesAsynchronously(forTimes: [NSValue(time: .zero)]) { _, cgImage, _, _, _ in
                continuation.resume(returning: cgImage.map(UIImage.init(cgImage:)))
            }
        }
    }
}

extension BotActivity {
    /// 读屏和状态格的名字（ADR-0044 的四个状态）。
    var text: String {
        switch self {
        case .idle: String(localized: .Chat.botStateIdle)
        case .working: String(localized: .Chat.botStateWorking)
        case .waiting: String(localized: .Chat.botStateWaiting)
        case .offline: String(localized: .Chat.botStateOffline)
        }
    }

    /// 界面测试用的稳定标识（跟着核心的键走，不跟语言走）。
    var identifier: String {
        switch self {
        case .idle: "idle"
        case .working: "working"
        case .waiting: "waiting"
        case .offline: "offline"
        }
    }
}
