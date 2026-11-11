import SwiftUI
import UIKit
import WontonClient

/// UITextView supplies per-link menus and accessibility; classification stays in RichLink from the core.
struct LinkedMessageText: UIViewRepresentable {
    let block: RichBlock
    let ink: Color
    let reserve: String?
    @ScaledMetric private var fontSize: CGFloat
    @ScaledMetric private var line: CGFloat
    @Environment(RoomPresenter.self) private var presenter

    init(block: RichBlock, ink: Color, reserve: String?) {
        self.block = block
        self.ink = ink
        self.reserve = reserve
        let style = block.kind == .heading ? WontonTheme.typography.itemTitle : WontonTheme.typography.body
        _fontSize = ScaledMetric(wrappedValue: style.size, relativeTo: style.relativeTo)
        _line = ScaledMetric(wrappedValue: style.line, relativeTo: style.relativeTo)
    }

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.delegate = context.coordinator
        view.isEditable = false
        view.isScrollEnabled = false
        view.backgroundColor = .clear
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return view
    }

    func updateUIView(_ view: UITextView, context: Context) {
        context.coordinator.parent = self
        view.linkTextAttributes = [.foregroundColor: UIColor(ink), .underlineStyle: NSUnderlineStyle.single.rawValue]
        let text = attributedText
        if !view.attributedText.isEqual(to: text) { view.attributedText = text }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        guard let width = proposal.width, width > 0 else { return nil }
        let bounds = uiView.attributedText.boundingRect(
            with: CGSize(width: width, height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil
        )
        let fittedWidth = min(width, ceil(bounds.width))
        return CGSize(width: fittedWidth, height: ceil(uiView.sizeThatFits(
            CGSize(width: fittedWidth, height: .greatestFiniteMagnitude)
        ).height))
    }

    func makeCoordinator() -> Coordinator { Coordinator(parent: self) }

    private var attributedText: NSAttributedString {
        let text = NSMutableAttributedString(string: "")
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = max(0, line - UIFont.systemFont(ofSize: fontSize).lineHeight)
        for span in block.spans {
            var font = span.code ? UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
                : UIFont.systemFont(ofSize: fontSize, weight: block.kind == .heading ? .medium : .regular)
            var traits = font.fontDescriptor.symbolicTraits
            if span.bold { traits.insert(.traitBold) }
            if span.italic { traits.insert(.traitItalic) }
            if let descriptor = font.fontDescriptor.withSymbolicTraits(traits) {
                font = UIFont(descriptor: descriptor, size: fontSize)
            }
            var attributes: [NSAttributedString.Key: Any] = [
                .font: font, .foregroundColor: UIColor(ink), .paragraphStyle: paragraph
            ]
            if span.strike { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            if span.code { attributes[.backgroundColor] = UIColor(ink.opacity(WontonTheme.ratio.codeFill)) }
            if let link = span.link, let url = URL(string: link.url) { attributes[.link] = url }
            text.append(NSAttributedString(string: span.text, attributes: attributes))
        }
        if let reserve {
            text.append(NSAttributedString(string: reserve, attributes: [
                .foregroundColor: UIColor.clear, .font: UIFont.preferredFont(forTextStyle: .caption1)
            ]))
        }
        return text
    }

    @MainActor
    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: LinkedMessageText
        init(parent: LinkedMessageText) { self.parent = parent }

        func textView(_ textView: UITextView, primaryActionFor textItem: UITextItem, defaultAction: UIAction) -> UIAction? {
            guard case let .link(url) = textItem.content, let link = link(for: url) else { return defaultAction }
            return UIAction { _ in AppNavigation.shared?.openLink(link) }
        }

        func textView(
            _ textView: UITextView, menuConfigurationFor textItem: UITextItem, defaultMenu: UIMenu
        ) -> UITextItem.MenuConfiguration? {
            guard case let .link(url) = textItem.content, link(for: url) != nil else { return nil }
            let copy = UIAction(title: String(localized: .messageCopy)) { _ in UIPasteboard.general.url = url }
            let browser = UIAction(title: String(localized: .messageLinkOpenBrowser)) { _ in
                UIApplication.shared.open(url)
            }
            let share = UIAction(title: String(localized: .viewerShare)) { [weak self] _ in
                self?.parent.presentShare(url)
            }
            return UITextItem.MenuConfiguration(preview: nil, menu: UIMenu(children: [copy, browser, share]))
        }

        private func link(for url: URL) -> RichLink? {
            parent.block.spans.compactMap(\.link).first { URL(string: $0.url) == url }
        }
    }

    private func presentShare(_ url: URL) {
        presenter.sharing = SharedFile(url: url)
    }
}
