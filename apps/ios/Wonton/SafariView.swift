import SafariServices
import SwiftUI

/// 外部网页和认不出的地址都在 App 内用 SFSafariViewController 打开，不切走 App（ADR-0065）。
struct SafariView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> SFSafariViewController {
        SFSafariViewController(url: url)
    }

    func updateUIViewController(_ controller: SFSafariViewController, context: Context) {}
}

/// `sheet(item:)` 用：地址变了换一张新的 Safari 页。
struct SafariTarget: Identifiable {
    let url: URL
    var id: String { url.absoluteString }
}
