import SwiftUI
import UIKit
import WontonClient
import XCTest

@testable import Wonton

/// 气泡末尾给时间留的透明占位（ADR-0109 第 3 节；#850 在 Android 修过同一处判定）：
/// 占位是单独一段 `.clear` 前景色接在正文后面，从它的起点切开就不算正文，不按空格或 NBSP 认
/// （时间自带普通空格，回执才是 NBSP）。
/// 时间压在占位上不压字，靠占位够宽、并且只在自己的开头折行；这里各量一条。
@MainActor
final class TimelineFooterReserveTests: XCTestCase {
    /// 占位是接在正文后面的一段透明字：按 `.clear` 样式切开，左边正好是正文、右边正好是占位。
    func testReserveIsOneClearRunAtTheEnd() throws {
        let block = paragraph(Self.longText)
        let reserve = MessageMeta.reserve(for: message(block))
        let text = RichTextInline.text(block, ink: .black, reserve: reserve)

        let runs = clearRuns(in: text)
        XCTAssertEqual(runs.count, 1, "占位要正好一段透明的字，不能和正文混成一段")
        let run = try XCTUnwrap(runs.first)
        XCTAssertEqual(run.upperBound, text.endIndex, "占位后面还有字")
        XCTAssertEqual(String(text[..<run.lowerBound].characters), Self.longText, "切开的地方不是正文末尾")
        XCTAssertEqual(String(text[run].characters), reserve, "切出来的不是占位本身")
    }

    /// 占位只在自己开头能折行：开头一个普通空格，其余空白全是不折行的 NBSP，时间是一整块。
    func testReserveOnlyBreaksAtItsStart() {
        for own in [false, true] {
            let reserve = MessageMeta.reserve(for: message(paragraph(Self.longText), own: own))
            XCTAssertEqual(reserve.first, Character(" "), "占位要从一个能折行的普通空格开始：\(reserve)")
            XCTAssertFalse(reserve.dropFirst().contains(" "), "占位中间还有能折行的空格，时间会被拆成两截：\(reserve)")
        }
    }

    /// 占位至少和压在它上面的那条一样宽：自己的消息还有回执图标，占位按图标也留了空白。
    func testReserveIsAtLeastAsWideAsTheMeta() throws {
        for own in [false, true] {
            let message = message(paragraph(Self.longText), own: own)
            let reserve = MessageMeta.reserve(for: message)
            let text = RichTextInline.text(paragraph(Self.longText), ink: .black, reserve: reserve)
            let run = try XCTUnwrap(clearRuns(in: text).last)
            // 下标切出来的是 AttributedSubstring，Text 只收 StringProtocol；按 run 的字体（.caption）量宽。
            let room = fittedWidth(Text(String(text[run].characters)).font(.caption))
            let meta = fittedWidth(MessageMeta(message: message, textIsSpoken: true))
            XCTAssertGreaterThanOrEqual(room, meta, "占位比时间窄，时间会压到正文上：\(reserve)")
        }
    }

    /// 带链接的块走另一条拼法（UITextView 的 NSAttributedString）：占位同样是末尾一段透明字。
    func testLinkedTextReserveIsAlsoAClearRunAtTheEnd() throws {
        let block = linkedBlock
        let reserve = MessageMeta.reserve(for: message(block))
        let text = LinkedMessageAttributes.text(block, ink: .black, fontSize: 16, line: 21, reserve: reserve)

        let run = try XCTUnwrap(clearRange(in: text))
        XCTAssertEqual(run.location + run.length, text.length, "占位后面还有字")
        XCTAssertEqual(text.attributedSubstring(from: run).string, reserve, "切出来的不是占位本身")
        XCTAssertEqual(
            text.attributedSubstring(from: NSRange(location: 0, length: run.location)).string,
            block.spans.map(\.text).joined(),
            "切开的地方不是正文末尾"
        )
    }
}

extension TimelineFooterReserveTests {
    private static let longText = String(repeating: "撑满一行也压不到字，", count: 6)

    private var linkedBlock: RichBlock {
        RichBlock(kind: .paragraph, depth: 0, quoted: false, spans: [
            richSpan("看一下"),
            richSpan("这个链接", link: RichLink(url: "https://example.com/", kind: .external)),
        ])
    }

    private func richSpan(_ text: String, link: RichLink? = nil) -> RichSpan {
        RichSpan(text: text, bold: false, italic: false, code: false, strike: false, link: link)
    }

    private func paragraph(_ text: String) -> RichBlock {
        RichBlock(kind: .paragraph, depth: 0, quoted: false, spans: [richSpan(text)])
    }

    /// 一条普通文字消息；`own` 时是自己的，占位里多留一截回执图标的空白。
    private func message(_ block: RichBlock, own: Bool = false) -> TimelineMessage {
        TimelineMessage(
            eventId: "$reserve:home.example",
            senderId: own ? "@owner:home.example" : "@ada:home.example",
            senderName: own ? "You" : "Ada",
            isOwn: own,
            timestampMs: 1_765_000_000_000,
            sendState: .sent,
            body: .text(body: block.spans.map(\.text).joined(), blocks: [block], files: []),
            edited: false,
            readBy: own ? ["@ada:home.example"] : [],
            reply: nil,
            sideChat: nil,
            preview: nil,
            actions: [],
            run: .alone,
            status: own ? .read : nil
        )
    }
}

extension TimelineFooterReserveTests {
    /// 透明字（`.clear` 前景色、alpha 0）的 run：和 `RichTextInline` 写占位用的是同一条样式。
    private func clearRuns(in text: AttributedString) -> [Range<AttributedString.Index>] {
        text.runs.compactMap { run in
            guard let color = run.foregroundColor, UIColor(color).cgColor.alpha == 0 else { return nil }
            return run.range
        }
    }

    /// 属性串里那段透明字的区间（`LinkedMessageAttributes` 用 `UIColor.clear`）。
    private func clearRange(in text: NSAttributedString) -> NSRange? {
        var found: NSRange?
        let all = NSRange(location: 0, length: text.length)
        text.enumerateAttribute(.foregroundColor, in: all) { value, range, _ in
            guard let color = value as? UIColor, color.cgColor.alpha == 0 else { return }
            XCTAssertNil(found, "透明字不止一段：\(text.string.debugDescription)")
            found = range
        }
        return found
    }

    /// 一行铺到底时的理想宽度：给 10 000 宽只排一行，`sizeThatFits` 报的是字自己的宽，不是给的宽。
    private func fittedWidth(_ view: some View) -> CGFloat {
        let host = UIHostingController(rootView: view)
        host.view.frame = CGRect(x: 0, y: 0, width: 10_000, height: 1_000)
        host.view.layoutIfNeeded()
        let width = host.view.sizeThatFits(CGSize(width: 10_000, height: 1_000)).width
        XCTAssertLessThan(width, 5_000, "量到的是一行的铺开宽，不是字自己的宽")
        return width
    }
}
