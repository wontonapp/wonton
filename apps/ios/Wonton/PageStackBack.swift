import SwiftUI
import UIKit

/// 插件页层的导航栈只留左边缘右滑返回（ADR-0095）。放在页面层导航栈的根视图上。
///
/// - iOS 26 起系统另给整片内容加了一个右滑返回手势（`interactiveContentPopGestureRecognizer`），会抢网页里的横滑；
///   页面层关掉它，网页里的横滑归网页。别的导航栈（聊天、设置）不受影响。
/// - 页面层盖在聊天根页面上：栈空和边缘返回拖到一半时要透出下面的聊天，所以导航栈自己的底色去掉。
///   iOS 17 没有 `containerBackground(_:for: .navigation)`，只能在 UIKit 这层改。
struct PageStackBack: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> Controller { Controller() }

    func updateUIViewController(_ controller: Controller, context: Context) {}

    final class Controller: UIViewController {
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
            if #available(iOS 26, *) {
                navigation.interactiveContentPopGestureRecognizer?.isEnabled = false
            }
        }
    }
}
