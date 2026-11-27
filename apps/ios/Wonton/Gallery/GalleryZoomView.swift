import SwiftUI
import UIKit

/// 大图页的一张图上手指做了什么，交给页面决定。
enum GalleryZoomEvent {
    case tap
    /// 没放大时往下拖：位移跟手，页面跟着缩小变淡。
    case dismissDrag(CGSize)
    /// 松手：`true` 是拖得够远或够快，关掉大图页。
    case dismissEnd(Bool)
    /// 长按住和松开（实况照片按住播放）。
    case hold(Bool)
}

/// 能缩放的图：双指捏合、双击放大到点的位置，放大后拖动看细节。缩放和拖动交给 `UIScrollView`，
/// SwiftUI 的手势拼不出跟手的双指缩放。
struct GalleryZoomView: UIViewRepresentable {
    let image: UIImage
    /// 放大着：外层的左右翻页先停下，手指留给拖动看细节。
    @Binding var zoomed: Bool
    let handle: (GalleryZoomEvent) -> Void

    func makeCoordinator() -> GalleryZoomCoordinator {
        GalleryZoomCoordinator()
    }

    func makeUIView(context: Context) -> GalleryZoomScrollView {
        let view = GalleryZoomScrollView()
        context.coordinator.attach(to: view)
        return view
    }

    func updateUIView(_ view: GalleryZoomScrollView, context: Context) {
        let zoomed = $zoomed
        context.coordinator.handle = handle
        context.coordinator.zoomChanged = { value in
            if zoomed.wrappedValue != value { zoomed.wrappedValue = value }
        }
        view.show(image)
    }
}

@MainActor
final class GalleryZoomCoordinator: NSObject, UIScrollViewDelegate, UIGestureRecognizerDelegate {
    var handle: (GalleryZoomEvent) -> Void = { _ in }
    var zoomChanged: (Bool) -> Void = { _ in }
    private weak var view: GalleryZoomScrollView?
    private var dismissPan: UIPanGestureRecognizer?

    func attach(to view: GalleryZoomScrollView) {
        self.view = view
        view.delegate = self
        let doubleTap = UITapGestureRecognizer(target: self, action: #selector(doubleTapped))
        doubleTap.numberOfTapsRequired = 2
        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped))
        tap.require(toFail: doubleTap)
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned))
        pan.delegate = self
        pan.maximumNumberOfTouches = 1
        let hold = UILongPressGestureRecognizer(target: self, action: #selector(held))
        hold.minimumPressDuration = 0.25
        for recognizer in [doubleTap, tap, pan, hold] { view.addGestureRecognizer(recognizer) }
        dismissPan = pan
    }

    func viewForZooming(in scrollView: UIScrollView) -> UIView? { view?.imageView }

    func scrollViewDidZoom(_ scrollView: UIScrollView) {
        view?.centerContent()
        zoomChanged(scrollView.zoomScale > scrollView.minimumZoomScale + 0.01)
    }

    @objc private func tapped() { handle(.tap) }

    @objc private func doubleTapped(_ tap: UITapGestureRecognizer) {
        guard let view else { return }
        if view.zoomScale > view.minimumZoomScale + 0.01 {
            view.setZoomScale(view.minimumZoomScale, animated: true)
        } else {
            let point = tap.location(in: view.imageView)
            let scale = min(view.maximumZoomScale, 2.5)
            let size = CGSize(width: view.bounds.width / scale, height: view.bounds.height / scale)
            let target = CGRect(x: point.x - size.width / 2, y: point.y - size.height / 2,
                                width: size.width, height: size.height)
            view.zoom(to: target, animated: true)
        }
    }

    @objc private func held(_ press: UILongPressGestureRecognizer) {
        switch press.state {
        case .began: handle(.hold(true))
        case .ended, .cancelled, .failed: handle(.hold(false))
        default: break
        }
    }

    /// 下滑关闭只在没放大、往下拖时开始；左右留给翻页，放大后留给拖动。
    func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
        guard recognizer === dismissPan, let view, let pan = dismissPan else { return true }
        guard view.zoomScale <= view.minimumZoomScale + 0.01 else { return false }
        let velocity = pan.velocity(in: view)
        return velocity.y > 0 && velocity.y > abs(velocity.x) * 1.2
    }

    @objc private func panned(_ pan: UIPanGestureRecognizer) {
        guard let view else { return }
        let translation = pan.translation(in: view.superview)
        switch pan.state {
        case .changed:
            handle(.dismissDrag(CGSize(width: translation.x, height: translation.y)))
        case .ended:
            let velocity = pan.velocity(in: view.superview).y
            handle(.dismissEnd(translation.y > 120 || velocity > 900))
        case .cancelled, .failed:
            handle(.dismissEnd(false))
        default:
            break
        }
    }
}

/// 布局回调：`UIScrollViewDelegate` 没有布局时机，转屏和首次布局靠它把图铺回去。
final class GalleryZoomScrollView: UIScrollView {
    let imageView = UIImageView()
    private var fitted = CGSize.zero

    override init(frame: CGRect) {
        super.init(frame: frame)
        addSubview(imageView)
        imageView.contentMode = .scaleAspectFit
        minimumZoomScale = 1
        maximumZoomScale = 5
        showsVerticalScrollIndicator = false
        showsHorizontalScrollIndicator = false
        contentInsetAdjustmentBehavior = .never
        bouncesZoom = true
        decelerationRate = .fast
        backgroundColor = .clear
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not used")
    }

    /// 换图：预览图到了顶替缩略图时，留着当前的缩放。
    func show(_ image: UIImage) {
        guard imageView.image !== image else { return }
        let ratio = { (image: UIImage) in image.size.width / max(image.size.height, 1) }
        let sameShape = imageView.image.map { abs(ratio($0) - ratio(image)) < 0.01 }
        imageView.image = image
        if sameShape != true || zoomScale <= minimumZoomScale + 0.01 {
            fitted = .zero
            setNeedsLayout()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        if fitted != bounds.size, bounds.width > 0, bounds.height > 0 {
            fitted = bounds.size
            fit()
        }
        centerContent()
    }

    private func fit() {
        guard let image = imageView.image, image.size.width > 0, image.size.height > 0 else { return }
        zoomScale = 1
        let scale = min(bounds.width / image.size.width, bounds.height / image.size.height)
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        imageView.frame = CGRect(origin: .zero, size: size)
        contentSize = size
    }

    /// 图比视口小的那个方向居中，`UIScrollView` 不会自己做。
    func centerContent() {
        let horizontal = max(0, (bounds.width - contentSize.width) / 2)
        let vertical = max(0, (bounds.height - contentSize.height) / 2)
        let inset = UIEdgeInsets(top: vertical, left: horizontal, bottom: vertical, right: horizontal)
        if contentInset != inset { contentInset = inset }
    }
}
