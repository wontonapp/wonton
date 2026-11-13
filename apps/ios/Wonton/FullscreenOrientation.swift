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
}
