import SwiftUI
import WontonClient

/// 排好版的消息文字。块和片段由核心给出，这里只管画。`ink` 是气泡里文字的颜色，链接和代码底色都跟着它。
struct RichTextView: View {
    let blocks: [RichBlock]
    let ink: Color
    /// 末尾接一段透明文字，给压在最后一行上的时间留位置。
    var trailingReserve: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
                RichBlockView(block: block, ink: ink, reserve: index == blocks.count - 1 ? trailingReserve : nil)
                    .padding(.top, index == 0 ? 0 : gap(after: blocks[index - 1], before: block))
            }
        }
        .tint(ink)
    }

    private func gap(after previous: RichBlock, before next: RichBlock) -> CGFloat {
        if case .listItem = previous.kind, case .listItem = next.kind {
            return WontonTheme.spacing.s4
        }
        return WontonTheme.spacing.s8
    }
}

/// 一段正文的文字：块里的片段接末尾那段透明占位。占位得能按 `.clear` 前景色认出来（ADR-0109 第 3 节），
/// 不靠空格或 NBSP 的写法：时间自带普通空格，回执才是 NBSP。
/// 带链接的块走 `LinkedMessageText` 的 UITextView，那边自己拼同一段 `.clear`，不走这里。
enum RichTextInline {
    static func text(_ block: RichBlock, ink: Color, reserve: String?) -> AttributedString {
        var text = spans(block, ink: ink)
        guard let reserve else { return text }
        var hidden = AttributedString(reserve)
        hidden.foregroundColor = .clear
        hidden.font = .caption
        text.append(hidden)
        return text
    }

    private static func spans(_ block: RichBlock, ink: Color) -> AttributedString {
        block.spans.reduce(into: AttributedString()) { result, span in
            var part = AttributedString(span.text)
            var intent: InlinePresentationIntent = []
            if span.bold { intent.insert(.stronglyEmphasized) }
            if span.italic { intent.insert(.emphasized) }
            if span.strike { intent.insert(.strikethrough) }
            if span.code {
                intent.insert(.code)
                part.backgroundColor = ink.opacity(WontonTheme.ratio.codeFill)
            }
            part.inlinePresentationIntent = intent
            if let link = span.link, let url = URL(string: link.url) {
                part.link = url
                part.underlineStyle = .single
            }
            result.append(part)
        }
    }
}

private struct RichBlockView: View {
    let block: RichBlock
    let ink: Color
    let reserve: String?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        content
            .padding(.leading, CGFloat(block.depth) * WontonTheme.spacing.s16)
            .padding(.leading, block.quoted ? WontonTheme.spacing.s12 : 0)
            .overlay(alignment: .leading) {
                if block.quoted {
                    Rectangle().fill(ink.opacity(WontonTheme.ratio.secondaryContent))
                        .frame(width: WontonTheme.size.quoteBar)
                }
            }
    }

    @ViewBuilder private var content: some View {
        switch block.kind {
        case .paragraph:
            inlineContent
        case .heading:
            inlineContent.wontonText(WontonTheme.typography.itemTitle)
        case let .listItem(marker):
            HStack(alignment: .firstTextBaseline, spacing: WontonTheme.spacing.s8) {
                Text(marker)
                inlineContent
            }
        case .code:
            CodeBlock(text: block.spans.first?.text ?? "", ink: ink)
        case let .table(rows):
            RichTable(rows: rows)
        case .rule:
            Rectangle().fill(Color(colors.separator)).frame(height: WontonTheme.size.hairline)
        }
    }

    @ViewBuilder private var inlineContent: some View {
        if block.spans.contains(where: { $0.link != nil }) {
            LinkedMessageText(block: block, ink: ink, reserve: reserve)
        } else {
            Text(RichTextInline.text(block, ink: ink, reserve: reserve))
        }
    }
}

/// 放得下时和内容一样宽，放不下才横向滚动，否则短内容也会把气泡撑满。
private struct Scrollable<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        ViewThatFits(in: .horizontal) {
            content
            ScrollView(.horizontal) { content }
        }
    }
}

private struct RichTable: View {
    let rows: [[String]]
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Scrollable {
            let spacing = WontonTheme.spacing
            Grid(alignment: .leading, horizontalSpacing: spacing.s16, verticalSpacing: spacing.s4) {
                ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                    GridRow {
                        ForEach(Array(row.enumerated()), id: \.offset) { _, cell in
                            Text(cell).fontWeight(index == 0 ? .semibold : nil).fixedSize()
                        }
                    }
                    if index == 0 {
                        Rectangle()
                            .fill(Color(colors.separator))
                            .frame(height: WontonTheme.size.hairline)
                            .gridCellUnsizedAxes(.horizontal)
                    }
                }
            }
        }
    }
}

/// 不折行，放不下才横向滚动；右上角的复制按钮不随滚动移动。
private struct CodeBlock: View {
    let text: String
    let ink: Color
    @State private var copied = false

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            Scrollable { code }
                // 左边距在滚动区外，文字滚起来左边也留白；按钮占右侧 48pt，图标居中正好是 12pt 边距。
                .padding(.leading, WontonTheme.spacing.s12)
                // 短代码块只有一行时按按钮的高度撑开，文字在块里垂直居中。
                .frame(minHeight: WontonTheme.size.target)
            WontonIconButton(
                icon: copied ? .check : .copy,
                label: copied
                    ? String(localized: .Message2.messageCodeCopied)
                    : String(localized: .Message0.messageCodeCopy),
                tint: ink
            ) {
                UIPasteboard.general.string = text
                copied = true
            }
            .task(id: copied) {
                guard copied else { return }
                try? await Task.sleep(for: .seconds(2))
                copied = false
            }
        }
        .background(
            ink.opacity(WontonTheme.ratio.codeFill),
            in: RoundedRectangle(cornerRadius: WontonTheme.radii.control)
        )
    }

    private var code: some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .monospaced()
            .fixedSize()
            .padding(.vertical, WontonTheme.spacing.s8)
    }
}
