import UIKit
import WontonClient

@MainActor
final class AppActivity: NSObject {
    private let home: HomeModel
    private var networkTask: Task<Void, Never>?

    init(home: HomeModel) {
        self.home = home
        super.init()
        NotificationCenter.default.addObserver(
            self, selector: #selector(foregrounded), name: UIApplication.didBecomeActiveNotification, object: nil
        )
        NotificationCenter.default.addObserver(
            self, selector: #selector(backgrounded), name: UIApplication.willResignActiveNotification, object: nil
        )
        networkTask = Task {
            for await network in DeviceNetwork.changes() { home.send(.home(action: .networkChanged(network: network))) }
        }
    }

    @objc private func foregrounded() { home.send(.home(action: .appForegrounded)) }
    @objc private func backgrounded() { home.send(.home(action: .appBackgrounded)) }

    deinit {
        networkTask?.cancel()
        NotificationCenter.default.removeObserver(self)
    }
}
