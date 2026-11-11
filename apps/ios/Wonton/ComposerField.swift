import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// 聊天输入框。SwiftUI 的 TextField 接不住剪贴板里的图片和文件（它只认文字），换成 UITextView：
/// 长按菜单的“粘贴”、键盘快捷键 Cmd+V 和键盘建议条上的粘贴芯片最后都走到 `ComposerTextView.paste(_:)`，
/// 在那里把要发的图片和文件截下来，交给上层走发图、发文件的流程。
struct ComposerField: View {
    @Binding var text: String
    @FocusState.Binding var focused: Bool
    var onPasteImage: ([NSItemProvider]) -> Void
    var onPasteFiles: ([URL]) -> Void
    var onSend: () -> Void

    @Environment(\.wontonColors) private var colors
    @ScaledMetric(relativeTo: .body) private var fontSize: CGFloat = WontonTheme.typography.body.size
    @ScaledMetric(relativeTo: .body) private var line: CGFloat = WontonTheme.typography.body.line

    var body: some View {
        ComposerTextField(
            text: $text,
            focused: $focused,
            onPasteImage: onPasteImage,
            onPasteFiles: onPasteFiles,
            onSend: onSend,
            fontSize: fontSize,
            line: line,
            colors: colors,
            placeholder: String(localized: .composerMessage)
        )
        .fixedSize(horizontal: false, vertical: true)
        .frame(minHeight: WontonTheme.size.composerField)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.composerField).fill(Color(colors.surfaceSubtle)))
    }
}

private struct ComposerTextField: UIViewRepresentable {
    @Binding var text: String
    @FocusState.Binding var focused: Bool
    var onPasteImage: ([NSItemProvider]) -> Void
    var onPasteFiles: ([URL]) -> Void
    var onSend: () -> Void
    var fontSize: CGFloat
    var line: CGFloat
    var colors: ThemeColors
    var placeholder: String

    func makeUIView(context: Context) -> ComposerTextView {
        let view = ComposerTextView()
        view.delegate = context.coordinator
        view.onPasteImage = onPasteImage
        view.onPasteFiles = onPasteFiles
        view.onSend = onSend
        view.placeholderLabel.text = placeholder
        view.accessibilityLabel = placeholder
        // 系统的粘贴建议（键盘上方那一枚）要声明接受什么才出现。
        view.pasteConfiguration = UIPasteConfiguration(acceptableTypeIdentifiers: [
            UTType.plainText.identifier, UTType.url.identifier, UTType.image.identifier, UTType.fileURL.identifier
        ])
        return view
    }

    func updateUIView(_ view: ComposerTextView, context: Context) {
        context.coordinator.parent = self
        view.onPasteImage = onPasteImage
        view.onPasteFiles = onPasteFiles
        view.onSend = onSend
        view.refreshStyle(fontSize: fontSize, line: line, colors: colors)
        // 输入法组词（marked text）期间 text 和绑定对不上，别动它，等上屏。
        if view.markedTextRange == nil, view.text != text { view.setText(text) }
        // 点了回复要把光标放进来；取消回复再回到普通状态。
        if focused, !view.isFirstResponder { view.becomeFirstResponder() }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: ComposerTextField

        init(parent: ComposerTextField) {
            self.parent = parent
        }

        func textViewDidChange(_ view: UITextView) {
            guard let view = view as? ComposerTextView else { return }
            parent.text = view.text
            view.placeholderLabel.isHidden = !view.text.isEmpty
            view.fitHeight()
        }

        func textViewDidBeginEditing(_ view: UITextView) {
            parent.focused = true
        }

        func textViewDidEndEditing(_ view: UITextView) {
            parent.focused = false
        }
    }
}

/// 接住系统“粘贴”的文本视图。一到六行，到顶再打字就滚动，和原 `TextField(axis: .vertical)` 一样。
final class ComposerTextView: UITextView {
    static let maxLines = 6

    var onPasteImage: (([NSItemProvider]) -> Void)?
    var onPasteFiles: (([URL]) -> Void)?
    var onSend: (() -> Void)?
    private var sendingReturnKeys: Set<UIKeyboardHIDUsage> = []

    // UIPress.key exists only for physical keys; software Return still goes through UITextInput unchanged.
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        var remaining = presses
        for press in presses {
            guard let key = press.key,
                  key.keyCode == .keyboardReturnOrEnter || key.keyCode == .keypadEnter,
                  key.modifierFlags.intersection([.shift, .control, .alternate, .command]).isEmpty,
                  markedTextRange == nil else { continue }
            remaining.remove(press)
            if sendingReturnKeys.insert(key.keyCode).inserted { onSend?() }
        }
        if !remaining.isEmpty { super.pressesBegan(remaining, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let remaining = removeSendingReturns(from: presses)
        if !remaining.isEmpty { super.pressesEnded(remaining, with: event) }
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let remaining = removeSendingReturns(from: presses)
        if !remaining.isEmpty { super.pressesCancelled(remaining, with: event) }
    }

    private func removeSendingReturns(from presses: Set<UIPress>) -> Set<UIPress> {
        Set(presses.filter { press in
            guard let key = press.key else { return true }
            return sendingReturnKeys.remove(key.keyCode) == nil
        })
    }

    let placeholderLabel = UILabel()
    private var placeholderTop: NSLayoutConstraint!
    private var contentLine: CGFloat = WontonTheme.typography.body.line
    private var fittedHeight: CGFloat = WontonTheme.size.composerField
    /// 设 `text` 会把 `typingAttributes` 重置成默认段落，存一份随时恢复。
    private var typingStyle: [NSAttributedString.Key: Any] = [:]

    override init(frame: CGRect, textContainer: NSTextContainer?) {
        super.init(frame: frame, textContainer: textContainer)
        backgroundColor = .clear
        // 文字贴着左缘:SwiftUI 那层的横向 padding 负责边距。参数在有些 SDK 上是可选的,属性在另一些上是,用参数加可选链接两边都编译。
        textContainer?.lineFragmentPadding = 0
        alwaysBounceVertical = false
        showsVerticalScrollIndicator = false
        setUpPlaceholder()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("Only created in code")
    }

    private func setUpPlaceholder() {
        placeholderLabel.translatesAutoresizingMaskIntoConstraints = false
        placeholderLabel.isAccessibilityElement = false
        addSubview(placeholderLabel)
        placeholderTop = placeholderLabel.topAnchor.constraint(equalTo: topAnchor)
        NSLayoutConstraint.activate([
            placeholderTop,
            placeholderLabel.leadingAnchor.constraint(equalTo: leadingAnchor),
            placeholderLabel.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor)
        ])
    }

    /// 字号跟着系统的字号走（ADR-0026），每次外观或文字变化都重设样式。
    func refreshStyle(fontSize: CGFloat, line: CGFloat, colors: ThemeColors) {
        let font = UIFont.systemFont(ofSize: fontSize)
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = max(0, line - font.lineHeight)
        let ink = UIColor(Color(colors.textPrimary))
        self.font = font
        textColor = ink
        tintColor = UIColor(Color(colors.primary))
        typingAttributes = [.font: font, .foregroundColor: ink, .paragraphStyle: paragraph]
        typingStyle = typingAttributes
        placeholderLabel.font = font
        placeholderLabel.textColor = UIColor(Color(colors.textSecondary))
        contentLine = line
        fitHeight()
    }

    /// 外部来的文字（草稿恢复、提示语追加载入）：整体重设样式，光标到末尾。
    func setText(_ text: String) {
        self.text = text
        selectedRange = NSRange(location: (text as NSString).length, length: 0)
        typingAttributes = typingStyle
        if markedTextRange == nil {
            textStorage.setAttributes(typingStyle, range: NSRange(location: 0, length: (text as NSString).length))
        }
        placeholderLabel.isHidden = text.isEmpty
        fitHeight()
    }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: fittedHeight)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        fitHeight()
    }

    /// 按内容定高：一行时居中垫空（最小高度 `size.composerField`），到 `maxLines` 行封顶改滚动。
    func fitHeight() {
        textContainerInset = .zero
        let content = sizeThatFits(CGSize(width: bounds.width, height: .greatestFiniteMagnitude)).height
        let box = min(
            max(WontonTheme.size.composerField, content + WontonTheme.spacing.s8 * 2),
            contentLine * CGFloat(Self.maxLines) + WontonTheme.spacing.s8 * 2
        )
        let pad = max(WontonTheme.spacing.s8, (box - content) / 2)
        textContainerInset = UIEdgeInsets(top: pad, left: 0, bottom: pad, right: 0)
        placeholderTop.constant = pad
        isScrollEnabled = content + WontonTheme.spacing.s8 * 2 > box + 0.5
        if fittedHeight != box {
            fittedHeight = box
            invalidateIntrinsicContentSize()
        }
    }

    // MARK: 粘贴

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        // 剪贴板里是要发的东西时，文本视图默认不给“粘贴”这一项（它只认文字）；打开它，长按菜单才出“粘贴”。
        if action == #selector(paste(_:)) {
            let pasteboard = UIPasteboard.general
            let hasFiles = pasteboard.itemProviders.contains {
                $0.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
            }
            if pasteboard.hasImages || hasFiles { return true }
        }
        return super.canPerformAction(action, withSender: sender)
    }

    override func paste(_ sender: Any?) {
        // 长按菜单的“粘贴”和 Cmd+V 都到这。文件优先：文件 App 里拷贝的图片文件同时带 file URL 和图片数据，
        // 发原文件而不是转码后的图。浏览器“拷贝图像”带的 https 地址不算文件，走图片。
        let files = UIPasteboard.general.urls?.filter(\.isFileURL) ?? []
        if !files.isEmpty {
            onPasteFiles?(files)
            return
        }
        if UIPasteboard.general.hasImages {
            onPasteImage?(UIPasteboard.general.itemProviders)
            return
        }
        super.paste(sender)
    }

    override func paste(itemProviders providers: [NSItemProvider]) {
        // 键盘建议条上的“粘贴”芯片走这个入口，不走 `paste(_:)`；两条入口做同一件事。
        let fileProviders = providers.filter { $0.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) }
        if !fileProviders.isEmpty {
            Task {
                let urls = await ComposerPaste.fileURLs(from: fileProviders)
                onPasteFiles?(urls)
            }
            return
        }
        if providers.contains(where: { $0.hasItemConformingToTypeIdentifier(UTType.image.identifier) }) {
            onPasteImage?(providers)
            return
        }
        super.paste(itemProviders: providers)
    }
}

/// 从剪贴板的 NSItemProvider 里取要发的东西的原始数据。
enum ComposerPaste {
    /// 图片的原始字节和类型。没有图片或读不出来时返回 nil，上层按“图片读不了”提示。
    /// 停在主线程,理由同 fileURLs:调用方在主线程拿到 NSItemProvider。
    @MainActor
    static func image(from providers: [NSItemProvider]) async -> (Data, UTType?)? {
        guard let provider = providers.first(where: { $0.hasItemConformingToTypeIdentifier(UTType.image.identifier) })
        else { return nil }
        let identifier = provider.registeredTypeIdentifiers.first { UTType($0)?.conforms(to: .image) == true }
            ?? UTType.image.identifier
        guard let data = await data(provider, identifier), !data.isEmpty else { return nil }
        return (data, UTType(identifier))
    }

    /// 文件 App“拷贝”出来的 file URL，可能多个。任一文件读不出时整批交给上层报错。
    /// 停在主线程:调用方在主线程拿到 NSItemProvider,Swift 6 不让非 Sendable 的它跨 isolation 传递。
    @MainActor
    static func fileURLs(from providers: [NSItemProvider]) async -> [URL] {
        var urls: [URL] = []
        for provider in providers where provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
            guard let data = await data(provider, UTType.fileURL.identifier),
                  let url = URL(dataRepresentation: data, relativeTo: nil), url.isFileURL else { return [] }
            urls.append(url)
        }
        return urls
    }

    @MainActor
    private static func data(_ provider: NSItemProvider, _ identifier: String) async -> Data? {
        await withCheckedContinuation { continuation in
            provider.loadDataRepresentation(forTypeIdentifier: identifier) { data, _ in
                continuation.resume(returning: data)
            }
        }
    }
}
