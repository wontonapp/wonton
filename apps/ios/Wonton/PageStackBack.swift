import SwiftUI
import UIKit

/// 盖在聊天根页面上的导航栈（插件页层、“全部应用”层，ADR-0095）。放在这种导航栈的根视图上。
///
/// - 这一层盖在聊天根页面上：栈空和边缘返回拖到一半时要透出下面的聊天，所以导航栈自己的底色去掉。
///   iOS 17 没有 `containerBackground(_:for: .navigation)`，只能在 UIKit 这层改。
/// - iOS 26 起系统另给整片内容加了一个右滑返回手势（`interactiveContentPopGestureRecognizer`），会抢网页里的横滑；
///   插件页层关掉它（`contentPop` 为 false），只留左边缘右滑，网页里的横滑归网页。别的导航栈（聊天、设置）不受影响。
struct PageStackBack: UIViewControllerRepresentable {
    var contentPop = false

    func makeUIViewController(context: Context) -> Controller { Controller(contentPop: contentPop) }

    func updateUIViewController(_ controller: Controller, context: Context) {}

    final class Controller: UIViewController {
        private let contentPop: Bool

        init(contentPop: Bool) {
            self.contentPop = contentPop
            super.init(nibName: nil, bundle: nil)
        }

        required init?(coder: NSCoder) { nil }

        override func didMove(toParent parent: UIViewController?) {
            super.didMove(toParent: parent)
            configure()
        }

        override func viewWillAppear(_ animated: Bool) {
            super.viewWillAppear(animated)
            configure()
        }

        private func configure() {
            view.backgroundColor = .clear
            parent?.view.backgroundColor = .clear
            guard let navigation = navigationController else { return }
            navigation.view.backgroundColor = .clear
            if #available(iOS 26, *), !contentPop {
                navigation.interactiveContentPopGestureRecognizer?.isEnabled = false
            }
        }
    }
}
