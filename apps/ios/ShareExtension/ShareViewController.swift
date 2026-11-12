import UIKit
import UniformTypeIdentifiers

/// 系统分享面板里的“云吞”（ADR-0097）：把分享的东西复制进主 App 的收件目录，再打开主 App；发不发、发去哪在主 App 的
/// 分享页定。不链接 Rust 核心，原因见 ADR 第 5 节。
final class ShareViewController: UIViewController {
    private let spinner = UIActivityIndicatorView(style: .large)

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        spinner.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(spinner)
        NSLayoutConstraint.activate([
            spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: view.centerYAnchor)
        ])
        spinner.startAnimating()
        let items = extensionContext?.inputItems.compactMap { $0 as? NSExtensionItem } ?? []
        Task { await deliver(items) }
    }

    private func deliver(_ items: [NSExtensionItem]) async {
        let token = UUID().uuidString.lowercased()
        let inbox: ShareInbox
        do {
            inbox = try ShareInbox(token: token)
        } catch {
            fail(.unreadable, detail: String(describing: error))
            return
        }
        do {
            try await inbox.write(items.flatMap { $0.attachments ?? [] })
        } catch {
            inbox.remove()
            fail(.unreadable, detail: String(describing: error))
            return
        }
        guard let url = URL(string: "wonton://share/\(token)"), await openApp(url) else {
            // 主 App 没打开就不会认领：删掉这次的收件目录，不假装成功。
            inbox.remove()
            fail(.cannotOpen, detail: nil)
            return
        }
        extensionContext?.completeRequest(returningItems: nil)
    }

    /// 沿应答链找到 UIApplication 打开主 App。这不是 Apple 文档里的做法（扩展的 `NSExtensionContext.open` 只给 Today
    /// 和 iMessage 扩展用），打不开时返回 false。
    private func openApp(_ url: URL) async -> Bool {
        var responder: UIResponder? = self
        while let current = responder {
            if let application = current as? UIApplication {
                return await application.open(url)
            }
            responder = current.next
        }
        return false
    }

    private enum Failure {
        case unreadable
        case cannotOpen
    }

    private func fail(_ failure: Failure, detail: String?) {
        spinner.stopAnimating()
        let (title, nextStep): (String, String) = switch failure {
        case .unreadable:
            (String(localized: .shareExtensionUnreadable), String(localized: .shareExtensionUnreadableNextStep))
        case .cannotOpen:
            (String(localized: .shareExtensionCannotOpen), String(localized: .shareExtensionCannotOpenNextStep))
        }
        let titleLabel = label(title, style: .headline, color: .label)
        let nextLabel = label(nextStep, style: .body, color: .secondaryLabel)
        var lines = [titleLabel, nextLabel]
        if let detail { lines.append(label(detail, style: .footnote, color: .tertiaryLabel)) }
        var close = UIButton.Configuration.filled()
        close.title = String(localized: .shareExtensionClose)
        let button = UIButton(configuration: close, primaryAction: UIAction { [weak self] _ in
            self?.extensionContext?.cancelRequest(withError: CocoaError(.userCancelled))
        })
        let stack = UIStackView(arrangedSubviews: lines + [button])
        stack.axis = .vertical
        stack.spacing = 12
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            stack.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor)
        ])
    }

    private func label(_ text: String, style: UIFont.TextStyle, color: UIColor) -> UILabel {
        let label = UILabel()
        label.text = text
        label.font = .preferredFont(forTextStyle: style)
        label.adjustsFontForContentSizeCategory = true
        label.textColor = color
        label.numberOfLines = 0
        label.textAlignment = .center
        return label
    }
}
