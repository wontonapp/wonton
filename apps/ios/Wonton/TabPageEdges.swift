import SwiftUI
import WebKit

/// 网页框够不到屏幕边时，留白处画网页自己贴边的那一行（列）像素，拉伸铺满：
/// 页面看起来延伸到了屏幕边，不出现 App 的底色。`theme-color` 只是一个颜色，
/// 和页面顶栏常常对不上（Navidrome 实测），所以取像素。
@MainActor
@Observable
final class TabPageEdges {
    private(set) var images: [Edge: UIImage] = [:]
    private(set) var base: UIColor?

    func sample(_ webView: WKWebView, edges: Set<Edge>) {
        let bounds = webView.bounds
        guard bounds.width >= 1, bounds.height >= 1 else { return }
        base = webView.underPageBackgroundColor
        for edge in edges {
            let configuration = WKSnapshotConfiguration()
            configuration.rect = Self.rect(edge, in: bounds)
            configuration.afterScreenUpdates = false
            webView.takeSnapshot(with: configuration) { [weak self] image, _ in
                self?.images[edge] = image
            }
        }
    }

    private static func rect(_ edge: Edge, in bounds: CGRect) -> CGRect {
        switch edge {
        case .top: CGRect(x: 0, y: 0, width: bounds.width, height: 1)
        case .bottom: CGRect(x: 0, y: bounds.height - 1, width: bounds.width, height: 1)
        case .leading: CGRect(x: 0, y: 0, width: 1, height: bounds.height)
        case .trailing: CGRect(x: bounds.width - 1, y: 0, width: 1, height: bounds.height)
        }
    }
}

/// 铺满整个屏幕（调用方负责 `ignoresSafeArea`）；`gaps` 是网页框到屏幕四边的距离。
struct TabPageBackdrop: View {
    let edges: TabPageEdges
    let gaps: EdgeInsets
    let fallback: Color

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let middle = max(0, size.height - gaps.top - gaps.bottom)
            ZStack(alignment: .topLeading) {
                fallback
                strip(.top).frame(width: size.width, height: gaps.top)
                strip(.bottom).frame(width: size.width, height: gaps.bottom)
                    .offset(y: size.height - gaps.bottom)
                strip(.leading).frame(width: gaps.leading, height: middle)
                    .offset(y: gaps.top)
                strip(.trailing).frame(width: gaps.trailing, height: middle)
                    .offset(x: size.width - gaps.trailing, y: gaps.top)
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    @ViewBuilder private func strip(_ edge: Edge) -> some View {
        if let image = edges.images[edge] {
            Image(uiImage: image).resizable()
        } else {
            Color.clear
        }
    }
}

extension EdgeInsets {
    /// 有留白、需要取像素的边。
    var sampledEdges: Set<Edge> {
        var edges = Set<Edge>()
        if top > 0 { edges.insert(.top) }
        if bottom > 0 { edges.insert(.bottom) }
        if leading > 0 { edges.insert(.leading) }
        if trailing > 0 { edges.insert(.trailing) }
        return edges
    }
}
