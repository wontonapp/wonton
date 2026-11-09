import SwiftUI
import WontonClient

/// 排好版的消息文字。块和片段由核心给出，这里只管画。`ink` 是气泡里文字的颜色，链接和代码底色都跟着它。
struct RichTextView: View {
    let blocks: [RichBlock]
    let ink: Color
    /// 末尾接一段透明文字，给压在最后一行上的时间留位置。
    var trailingReserve: String?

    var body: some View {
        let links = linkMap
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
                RichBlockView(block: block, ink: ink, reserve: index == blocks.count - 1 ? trailingReserve : nil)
                    .padding(.top, index == 0 ? 0 : gap(after: blocks[index - 1], before: block))
            }
        }
        .tint(ink)
        .environment(\.openURL, OpenURLAction { url in
            guard let link = links[url] else { return .systemAction }
            AppNavigation.shared?.openLink(link)
            return .handled
        })
    }

    /// 这一块文字里每个地址的类别：AttributedString 的链接只带地址，点开时按地址找回类别。
    private var linkMap: [URL: RichLink] {
        var links: [URL: RichLink] = [:]
        for span in blocks.flatMap(\.spans) {
            guard let link = span.link, let url = URL(string: link.url) else { continue }
            links[url] = link
        }
        return links
    }

    private func gap(after previous: RichBlock, before next: RichBlock) -> CGFloat {
        if case .listItem = previous.kind, case .listItem = next.kind {
            return WontonTheme.spacing.s4
        }
        return WontonTheme.spacing.s8
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
            Text(inline)
        case .heading:
            Text(inline).wontonText(WontonTheme.typography.itemTitle)
        case let .listItem(marker):
            HStack(alignment: .firstTextBaseline, spacing: WontonTheme.spacing.s8) {
                Text(marker)
                Text(inline)
            }
        case .code:
            CodeBlock(text: block.spans.first?.text ?? "", ink: ink)
        case let .table(rows):
            RichTable(rows: rows)
        case .rule:
            Rectangle().fill(Color(colors.separator)).frame(height: WontonTheme.size.hairline)
        }
    }

    private var inline: AttributedString {
        var text = spans
        if let reserve {
            var hidden = AttributedString(reserve)
            hidden.foregroundColor = .clear
            hidden.font = .caption
            text.append(hidden)
        }
        return text
    }

    private var spans: AttributedString {
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

private struct CodeBlock: View {
    let text: String
    let ink: Color

    var body: some View {
        Scrollable { code }
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
            .padding(.horizontal, WontonTheme.spacing.s12)
            .padding(.vertical, WontonTheme.spacing.s8)
    }
}
