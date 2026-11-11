import SwiftUI
import UIKit

@MainActor
final class FullscreenOrientation: NSObject, UIApplicationDelegate, ObservableObject {
    private var fullscreen = false

    func application(
        _ application: UIApplication, supportedInterfaceOrientationsFor window: UIWindow?
    ) -> UIInterfaceOrientationMask {
        UIDevice.current.userInterfaceIdiom == .pad || fullscreen ? .allButUpsideDown : .portrait
    }

    func update(fullscreen: Bool, failed: @escaping (Error) -> Void) {
        self.fullscreen = fullscreen
        for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
            guard scene.activationState == .foregroundActive else { continue }
            var controller = scene.keyWindow?.rootViewController
            while let presented = controller?.presentedViewController { controller = presented }
            controller?.setNeedsUpdateOfSupportedInterfaceOrientations()
            if !fullscreen, UIDevice.current.userInterfaceIdiom != .pad {
                scene.requestGeometryUpdate(.iOS(interfaceOrientations: .portrait), errorHandler: failed)
            }
        }
    }

    // 远程通知的注册结果只有一个 delegate 收得到（本类），转给推送模型。
    func application(
        _ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        PushModel.shared?.registered(token: deviceToken)
    }

    func application(
        _ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        PushModel.shared?.registrationFailed(error)
    }
}
