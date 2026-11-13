import UIKit.UIGestureRecognizerSubclass
import XCTest
@testable import Wonton

/// 面板网页上的拖动（ADR-0052）：只有从网页露出的那条边附近起手才拖面板，页面里面起手的归网页。
/// 网页的主滚动视图一直“在头上”的页面（全屏固定浮层、自己滚动的容器）靠滚动位置分不出来。
@MainActor
final class PanelPagePullTests: XCTestCase {
    private var window: UIWindow!
    private var scrollView: UIScrollView!
    private var moved: [CGFloat] = []
    private var ended = 0

    override func setUp() async throws {
        window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        scrollView = UIScrollView(frame: window.bounds)
        scrollView.contentInsetAdjustmentBehavior = .never
        scrollView.contentSize = CGSize(width: 390, height: 3000)
        window.addSubview(scrollView)
        window.isHidden = false
        moved = []
        ended = 0
    }

    override func tearDown() async throws {
        window.isHidden = true
        window = nil
        scrollView = nil
    }

    func testDragStartedInsideThePageLeavesThePanelAlone() {
        let anchor = pageAnchor(edge: .top)
        drag(anchor, fromY: 400, by: 150)
        XCTAssertEqual(moved, [], "A pull that started inside the page moved the panel")
        XCTAssertEqual(ended, 0)
    }

    func testDragStartedAtTheVisibleTopMovesThePanel() {
        let anchor = pageAnchor(edge: .top)
        drag(anchor, fromY: visible.minY + 10, by: 150)
        XCTAssertEqual(moved, [150])
        XCTAssertEqual(ended, 1)
    }

    func testTopZoneStartsBelowTheCoveredStrip() {
        let anchor = pageAnchor(edge: .top, obscured: UIEdgeInsets(top: 200, left: 0, bottom: 0, right: 0))
        let zone = WontonTheme.gesture.pageEdgeZone
        drag(anchor, fromY: 200 + zone + 20, by: 150)
        XCTAssertEqual(moved, [], "The zone was measured from the frame, not from the visible top")
        drag(anchor, fromY: 200 + zone - 4, by: 150)
        XCTAssertEqual(moved, [150])
    }

    func testBottomEdgeOnlyTakesDragsFromTheVisibleBottom() {
        let anchor = pageAnchor(edge: .bottom, obscured: UIEdgeInsets(top: 0, left: 0, bottom: 300, right: 0))
        scrollView.contentOffset.y = scrollView.contentSize.height + 300 - scrollView.bounds.height
        drag(anchor, fromY: 300, by: -150)
        XCTAssertEqual(moved, [], "A push that started inside the page moved the panel")
        drag(anchor, fromY: 844 - 300 - 10, by: -150)
        XCTAssertEqual(moved, [150])
    }

    func testChatListPullsFromAnywhere() {
        let anchor = PullAnchor(attachesToAncestor: false, edge: .top)
        wire(anchor)
        drag(anchor, fromY: 400, by: 150)
        XCTAssertEqual(moved, [150], "The chat list has no start zone")
    }

    func testZoneGeometry() {
        let visible = CGRect(x: 0, y: 100, width: 390, height: 600)
        let zone = WontonTheme.gesture.pageEdgeZone
        XCTAssertTrue(PagePullZone.contains(CGPoint(x: 10, y: 100 + zone), visible: visible, edge: .top))
        XCTAssertFalse(PagePullZone.contains(CGPoint(x: 10, y: 101 + zone), visible: visible, edge: .top))
        XCTAssertTrue(PagePullZone.contains(CGPoint(x: 10, y: 700 - zone), visible: visible, edge: .bottom))
        XCTAssertFalse(PagePullZone.contains(CGPoint(x: 10, y: 699 - zone), visible: visible, edge: .bottom))
    }

    // MARK: -

    /// 测试宿主的窗口可能带安全区：起手位置按看得见的那一块算。
    private var visible: CGRect { window.bounds.inset(by: window.safeAreaInsets) }

    private func pageAnchor(edge: PanelEdge, obscured: UIEdgeInsets = .zero) -> PullAnchor {
        let anchor = PullAnchor(attachesToAncestor: false, edge: edge)
        anchor.startZone = PagePullZone(obscured: obscured)
        wire(anchor)
        return anchor
    }

    private func wire(_ anchor: PullAnchor) {
        anchor.changed = { [unowned self] in moved.append($0) }
        anchor.ended = { [unowned self] _, _ in ended += 1 }
        anchor.attach(to: scrollView)
    }

    /// 一次慢拖：认出来（手指已经走了 10pt）、拖到位、松手。
    private func drag(_ anchor: PullAnchor, fromY start: CGFloat, by distance: CGFloat) {
        let direction: CGFloat = distance < 0 ? -1 : 1
        let pan = StubPan()
        pan.start = CGPoint(x: 195, y: start)
        pan.speed = CGPoint(x: 0, y: direction * 300)
        pan.moved = CGPoint(x: 0, y: direction * 10)
        pan.phase = .began
        anchor.panned(pan)
        pan.moved = CGPoint(x: 0, y: distance)
        pan.phase = .changed
        anchor.panned(pan)
        pan.phase = .ended
        anchor.panned(pan)
    }
}

/// 单元测试里造不出真的触摸：按给定的起点、位移和速度回答，位置按窗口坐标换算。
private final class StubPan: UIPanGestureRecognizer {
    var phase = UIGestureRecognizer.State.possible
    var start = CGPoint.zero
    var moved = CGPoint.zero
    var speed = CGPoint.zero

    override var state: UIGestureRecognizer.State {
        get { phase }
        set { phase = newValue }
    }

    override func translation(in view: UIView?) -> CGPoint { moved }

    override func velocity(in view: UIView?) -> CGPoint { speed }

    override func location(in view: UIView?) -> CGPoint {
        let point = CGPoint(x: start.x + moved.x, y: start.y + moved.y)
        return view?.convert(point, from: nil) ?? point
    }
}
