import SwiftUI
import WontonClient

extension MessageRun {
    /// 上面、下面是不是贴着同一组的另一条。不认识的取值当作自成一组（ADR-0109）。
    var joinsPrevious: Bool { self == .middle || self == .last }
    var joinsNext: Bool { self == .first || self == .middle }
    /// 组首写名字，组尾画尾巴和头像。
    var startsRun: Bool { !joinsPrevious }
    var endsRun: Bool { !joinsNext }
}

/// 气泡的外形（ADR-0109 第 2 节）：组内贴近发送者一侧的角是小圆角，组尾在发送者一侧的下角伸出尾巴。
///
/// 发送者一侧常留一条和尾巴等宽的边，同一组的气泡才对得齐；内容的衬垫要把这条边让出来。
struct MessageBubbleShape: InsettableShape {
    private let joinsPrevious: Bool
    private let joinsNext: Bool
    private let isOwn: Bool
    private var inset: CGFloat = 0

    init(run: MessageRun, isOwn: Bool) {
        joinsPrevious = run.joinsPrevious
        joinsNext = run.joinsNext
        self.isOwn = isOwn
    }

    func inset(by amount: CGFloat) -> MessageBubbleShape {
        var shape = self
        shape.inset += amount
        return shape
    }

    func path(in rect: CGRect) -> Path {
        let rect = rect.insetBy(dx: inset, dy: inset)
        let tail = WontonTheme.size.bubbleTail
        // 先按自己的消息（发送者在右）画，别人的左右翻过来。
        let body = CGRect(x: rect.minX, y: rect.minY, width: max(0, rect.width - tail), height: rect.height)
        let cap = min(body.width, body.height) / 2
        let outer = min(WontonTheme.radii.bubble, cap)
        let top = min(joinsPrevious ? WontonTheme.radii.bubbleJoined : WontonTheme.radii.bubble, cap)
        let bottom = min(joinsNext ? WontonTheme.radii.bubbleJoined : WontonTheme.radii.bubble, cap)

        var path = Path()
        path.move(to: CGPoint(x: body.minX + outer, y: body.minY))
        path.addLine(to: CGPoint(x: body.maxX - top, y: body.minY))
        path.addArc(
            tangent1End: CGPoint(x: body.maxX, y: body.minY),
            tangent2End: CGPoint(x: body.maxX, y: body.minY + top), radius: top
        )
        if !joinsNext {
            // 尾巴（ADR-0109 第 9 节）：侧边在底边上方 `bubbleTailRise` 处向外凹着弯出去，尖伸出 `bubbleTail`；
            // 尖的下缘鼓着收回底边，不是一条直线。
            let rise = WontonTheme.size.bubbleTailRise
            let tip = CGPoint(x: body.maxX + tail, y: body.maxY - 1)
            path.addLine(to: CGPoint(x: body.maxX, y: max(body.minY + top, body.maxY - rise)))
            path.addQuadCurve(to: tip, control: CGPoint(x: body.maxX, y: body.maxY - 2))
            path.addQuadCurve(
                to: CGPoint(x: max(body.minX + outer, body.maxX - rise), y: body.maxY),
                control: CGPoint(x: body.maxX, y: body.maxY)
            )
        } else {
            path.addLine(to: CGPoint(x: body.maxX, y: body.maxY - bottom))
            path.addArc(
                tangent1End: CGPoint(x: body.maxX, y: body.maxY),
                tangent2End: CGPoint(x: body.maxX - bottom, y: body.maxY), radius: bottom
            )
        }
        path.addLine(to: CGPoint(x: body.minX + outer, y: body.maxY))
        path.addArc(
            tangent1End: CGPoint(x: body.minX, y: body.maxY),
            tangent2End: CGPoint(x: body.minX, y: body.maxY - outer), radius: outer
        )
        path.addLine(to: CGPoint(x: body.minX, y: body.minY + outer))
        path.addArc(
            tangent1End: CGPoint(x: body.minX, y: body.minY),
            tangent2End: CGPoint(x: body.minX + outer, y: body.minY), radius: outer
        )
        path.closeSubpath()
        guard !isOwn else { return path }
        return path.applying(CGAffineTransform(a: -1, b: 0, c: 0, d: 1, tx: rect.minX + rect.maxX, ty: 0))
    }
}
