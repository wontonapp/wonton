import Photos
import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// 图片全屏查看器（消息里的原图）：双指缩放、双击放大还原、放大后拖动、下滑关闭；
/// 顶栏上保存进相册和分享（ADR-0026：相册和分享面板走系统能力）。保存和分享都用
/// 下载到的原图字节，不经 UIImage 重新编码，动图存出来还是动图。
struct ImageViewer: View {
    let image: ImageContent
    let core: ClientCore

    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.wontonColors) private var colors
    @State private var original: LoadOutcome?
    @State private var attempt = 0
    @State private var share: SharedFile?
    @State private var saveProblem: SaveProblem?
    @State private var justSaved = false
    /// 只有真去过了系统设置,回来才需要重新核对相册权限;权限弹窗本身也会让 scene 变 active。
    @State private var openedSettings = false
    /// 下滑关闭的进度：背景随之变透明，露出后面的房间。
    @State private var dragDimming: CGFloat = 0

    var body: some View {
        NavigationStack {
            Group {
                switch original {
                case let .loaded(loaded):
                    viewer(loaded)
                case let .failed(problem):
                    loadProblem(problem)
                case nil:
                    // loadMedia 不带进度回调，原图在下载时只有不确定的进度。
                    ProgressView().tint(.white)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.black.opacity(1 - dragDimming).ignoresSafeArea())
            .navigationTitle(image.filename)
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbarBackground(Color.black.opacity(1 - dragDimming), for: .navigationBar)
            .toolbarBackground(.visible, for: .navigationBar)
            .toolbar { actions }
        }
        .task(id: attempt) {
            original = nil
            original = await Self.load(image.media, from: core)
        }
        .sheet(item: $share) { ShareSheet(url: $0.url) }
        .onChange(of: scenePhase) { _, phase in
            // 从系统设置回来后相册权限可能变了:给过权限就清掉"没权限"提示,还是没给就留着如实显示。
            if phase == .active, openedSettings {
                openedSettings = false
                if saveProblem?.isDenied == true {
                    let status = PHPhotoLibrary.authorizationStatus(for: .addOnly)
                    if status == .authorized || status == .limited { saveProblem = nil }
                }
            }
        }
    }

    private func viewer(_ loaded: Original) -> some View {
        ZoomableImage(image: loaded.image, name: image.filename, dimming: $dragDimming) { dismiss() }
            .overlay(alignment: .bottom) {
                if let saveProblem {
                    saveNotice(saveProblem)
                        .padding(WontonTheme.spacing.pageInset)
                }
            }
    }

    @ToolbarContentBuilder private var actions: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Button { dismiss() } label: { WontonIcon.close.view() }
                .accessibilityLabel(Text(.Common.commonClose))
        }
        if case let .loaded(loaded) = original {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button { save(loaded) } label: {
                    (justSaved ? WontonIcon.checkCircleFill : WontonIcon.download).view()
                }
                .accessibilityLabel(Text(.Viewer.viewerSavePhoto))
                Button { share(loaded) } label: { WontonIcon.share.view() }
                    .accessibilityLabel(Text(.Viewer.viewerShare))
            }
        }
    }

    private func loadProblem(_ problem: Problem) -> some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            WontonNotice(icon: .imageBroken, title: problem.title, lines: problem.nextStep.map { [$0] } ?? []) {
                if let detail = problem.detail { ProblemDetail(text: detail) }
            }
            Button(.Common.commonRetry) { attempt += 1 }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
        }
        .padding(WontonTheme.spacing.pageInset)
    }

    private func saveNotice(_ saveProblem: SaveProblem) -> some View {
        WontonNotice(
            icon: .warning,
            title: saveProblem.problem.title,
            lines: saveProblem.problem.nextStep.map { [$0] } ?? []
        ) {
            if let detail = saveProblem.problem.detail { ProblemDetail(text: detail) }
            if saveProblem.isDenied {
                Button(.Common.commonOpenSettings) {
                    openedSettings = true
                    if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            HStack {
                Spacer(minLength: 0)
                Button { self.saveProblem = nil } label: {
                    WontonIcon.close.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(String(localized: .Common.commonClose))
            }
        }
    }

    /// 存原图。performChanges 的变更块会被 Photos 派到自己的队列上,不能带 MainActor 隔离,
    /// 否则 swift_task_checkIsolated 直接崩(实测 iOS 26.5 模拟器);所以写库这一步放 nonisolated 函数里。
    nonisolated private static func writeToLibrary(data: Data) async throws {
        try await PHPhotoLibrary.shared().performChanges {
            let request = PHAssetCreationRequest.forAsset()
            request.addResource(with: .photo, data: data, options: nil)
        }
    }

    private func save(_ original: Original) {
        saveProblem = nil
        Task {
            let status = await PHPhotoLibrary.requestAuthorization(for: .addOnly)
            guard status == .authorized || status == .limited else {
                saveProblem = SaveProblem(
                    problem: Problem(
                        title: String(localized: .Viewer.viewerPhotoPermission),
                        nextStep: String(localized: .Viewer.viewerPhotoPermissionHint)
                    ),
                    isDenied: true
                )
                return
            }
            do {
                try await Self.writeToLibrary(data: original.data)
                justSaved = true
                try? await Task.sleep(for: .seconds(2))
                justSaved = false
            } catch {
                saveProblem = SaveProblem(
                    problem: Problem(
                        title: String(localized: .Viewer.viewerSaveFailed),
                        nextStep: error.localizedDescription
                    ),
                    isDenied: false
                )
            }
        }
    }

    private func share(_ original: Original) {
        do {
            share = SharedFile(url: try Self.temporaryImageURL(image, data: original.data))
        } catch {
            saveProblem = SaveProblem(
                problem: Problem(
                    title: String(localized: .Viewer.viewerShareFailed),
                    nextStep: error.localizedDescription
                ),
                isDenied: false
            )
        }
    }

    /// 原图字节写进临时文件再交给分享面板：动图按动图分享，收到的就是聊天里的原文件。
    private static func temporaryImageURL(_ image: ImageContent, data: Data) throws -> URL {
        var name = (image.filename as NSString).lastPathComponent
        if name.isEmpty || name == "." || name == ".." { name = "image" }
        if (name as NSString).pathExtension.isEmpty, let mime = image.mimeType,
           let type = UTType(mimeType: mime), let ext = type.preferredFilenameExtension {
            name += "." + ext
        }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(name)
        try data.write(to: url)
        return url
    }

    private static func load(_ media: MediaHandle, from core: ClientCore) async -> LoadOutcome {
        do {
            let data = try await core.loadMedia(media: media)
            guard let image = UIImage(data: data) else {
                return .failed(Problem(title: String(localized: .Message0.messageImageFormatUnsupported)))
            }
            return .loaded(Original(image: image, data: data))
        } catch {
            return .failed(error.problem)
        }
    }
}

/// 下载的结果：原图（字节留着保存和分享用）或按类别的问题。
private enum LoadOutcome {
    case loaded(Original)
    case failed(Problem)
}

/// 下载完的原图：字节留着，保存和分享都用它。
private struct Original {
    let image: UIImage
    let data: Data
}

/// 保存或分享没做成的问题：`isDenied` 是没权限，从系统设置回来时提示就不再成立。
private struct SaveProblem {
    let problem: Problem
    let isDenied: Bool
}

/// 缩放和拖动交给 UIScrollView：SwiftUI 的手势拼不出跟手的双指缩放。
private struct ZoomableImage: UIViewRepresentable {
    let image: UIImage
    let name: String
    @Binding var dimming: CGFloat
    let onClose: () -> Void

    func makeCoordinator() -> ZoomableCoordinator {
        ZoomableCoordinator(image: image, name: name, dimming: $dimming, onClose: onClose)
    }

    func makeUIView(context: Context) -> ZoomScrollView {
        context.coordinator.makeScrollView()
    }

    func updateUIView(_ scrollView: ZoomScrollView, context: Context) {
        context.coordinator.update(dimming: $dimming, onClose: onClose)
    }
}

@MainActor
final class ZoomableCoordinator: NSObject, UIScrollViewDelegate, UIGestureRecognizerDelegate {
    private let imageView = UIImageView()
    private let image: UIImage
    private var dimming: Binding<CGFloat>
    private var onClose: () -> Void
    private weak var scrollView: ZoomScrollView?
    /// 下滑关闭进行中：图片的位置归手势管，布局和居中先让开。
    private var isDraggingToClose = false

    init(image: UIImage, name: String, dimming: Binding<CGFloat>, onClose: @escaping () -> Void) {
        self.image = image
        self.dimming = dimming
        self.onClose = onClose
        super.init()
        imageView.image = image
        imageView.contentMode = .scaleAspectFit
        imageView.isAccessibilityElement = true
        if !name.isEmpty { imageView.accessibilityLabel = name }
    }

    func makeScrollView() -> ZoomScrollView {
        let scrollView = ZoomScrollView()
        scrollView.delegate = self
        scrollView.minimumZoomScale = 1
        scrollView.showsVerticalScrollIndicator = false
        scrollView.showsHorizontalScrollIndicator = false
        scrollView.addSubview(imageView)

        let doubleTap = UITapGestureRecognizer(target: self, action: #selector(doubleTapped(_:)))
        doubleTap.numberOfTapsRequired = 2
        scrollView.addGestureRecognizer(doubleTap)
        let dragToClose = UIPanGestureRecognizer(target: self, action: #selector(draggedToClose(_:)))
        dragToClose.delegate = self
        scrollView.addGestureRecognizer(dragToClose)
        // 两个 pan 不能同时识别：先让下滑关闭判定，放大时它在 shouldBegin 里立即失败，滚动照常。
        scrollView.panGestureRecognizer.require(toFail: dragToClose)

        scrollView.onLayoutSubviews = { [weak self] in self?.fitToBounds() }
        self.scrollView = scrollView
        return scrollView
    }

    func update(dimming: Binding<CGFloat>, onClose: @escaping () -> Void) {
        self.dimming = dimming
        self.onClose = onClose
    }

    func viewForZooming(in scrollView: UIScrollView) -> UIView? { imageView }

    func scrollViewDidZoom(_ scrollView: UIScrollView) {
        centerIfNeeded()
    }

    @objc private func doubleTapped(_ tap: UITapGestureRecognizer) {
        guard let scrollView else { return }
        if scrollView.zoomScale > scrollView.minimumZoomScale + 0.01 {
            scrollView.setZoomScale(scrollView.minimumZoomScale, animated: true)
            return
        }
        // 双击放大到 2.5 倍，小图放不到就到上限。
        let scale = min(2.5, scrollView.maximumZoomScale)
        let side = scrollView.bounds.width / scale
        let center = tap.location(in: imageView)
        scrollView.zoom(
            to: CGRect(x: center.x - side / 2, y: center.y - side / 2, width: side, height: side),
            animated: true
        )
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard let pan = gestureRecognizer as? UIPanGestureRecognizer, let scrollView else { return false }
        // 放大后拖动是滚动；只在原大小时往下拖才关。
        if scrollView.zoomScale > scrollView.minimumZoomScale + 0.01 { return false }
        let velocity = pan.velocity(in: scrollView)
        return velocity.y > 0 && abs(velocity.y) > abs(velocity.x)
    }

    @objc private func draggedToClose(_ pan: UIPanGestureRecognizer) {
        guard let scrollView else { return }
        switch pan.state {
        case .began:
            isDraggingToClose = true
        case .changed:
            let translation = pan.translation(in: scrollView)
            pan.setTranslation(.zero, in: scrollView)
            // 只有往下是跟手的，往上和往横里都有阻尼。
            imageView.frame.origin.x += translation.x * 0.4
            imageView.frame.origin.y += translation.y > 0 ? translation.y : translation.y * 0.4
            dimming.wrappedValue = min(1, max(0, imageView.frame.origin.y / 240))
        case .ended:
            let velocity = pan.velocity(in: scrollView).y
            if imageView.frame.origin.y > 130 || velocity > 900 {
                dragOut()
            } else {
                springBack()
            }
        default:
            springBack()
        }
    }

    /// 手一松拖出屏幕再关，关的动画交给 fullScreenCover 自己。
    private func dragOut() {
        guard let scrollView else { return onClose() }
        UIView.animate(withDuration: 0.2, animations: {
            self.imageView.frame.origin.y = scrollView.frame.height
        }, completion: { _ in
            self.onClose()
        })
    }

    private func springBack() {
        UIView.animate(
            withDuration: 0.3, delay: 0,
            usingSpringWithDamping: 0.8, initialSpringVelocity: 0, options: []
        ) {
            self.imageView.frame.origin = .zero
            self.centerIfNeeded()
        } completion: { _ in
            self.isDraggingToClose = false
        }
        withAnimation(.easeOut(duration: 0.3)) { dimming.wrappedValue = 0 }
    }

    /// 视口变了（转屏、第一次布局）：原大小时图片铺满视口，上限到像素一比一，封顶 8 倍，再大也就糊了。
    private func fitToBounds() {
        guard let scrollView, !isDraggingToClose, scrollView.bounds.width > 0,
              abs(scrollView.zoomScale - scrollView.minimumZoomScale) < 0.01 else { return }
        imageView.frame = CGRect(origin: .zero, size: scrollView.bounds.size)
        scrollView.contentSize = imageView.frame.size
        let pixelFit = image.size.width * image.scale / scrollView.bounds.width
        scrollView.maximumZoomScale = max(3, min(pixelFit, 8))
    }

    /// 缩放后图片比视口小的时候居中，UIScrollView 不会自己做。
    private func centerIfNeeded() {
        guard let scrollView, !isDraggingToClose else { return }
        let bounds = scrollView.bounds.size
        let size = imageView.frame.size
        var origin = imageView.frame.origin
        origin.x = size.width < bounds.width ? (bounds.width - size.width) / 2 : 0
        origin.y = size.height < bounds.height ? (bounds.height - size.height) / 2 : 0
        imageView.frame.origin = origin
    }
}

/// 布局回调：UIScrollViewDelegate 没有布局时机，转屏和首次布局靠它把图片铺回去。
final class ZoomScrollView: UIScrollView {
    var onLayoutSubviews: (() -> Void)?

    override func layoutSubviews() {
        super.layoutSubviews()
        onLayoutSubviews?()
    }
}
