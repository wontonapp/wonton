import os
import UIKit
import WontonClient

/// 前后台和网页加载的时间轴（`log stream --predicate` 里看，和核心的 `revalidate` 日志对时刻）。
/// 只发 signpost，不改行为。
enum AppTrace {
    private static let signposter = OSSignposter(subsystem: "app.wonton", category: "page")

    static func event(_ name: StaticString, _ detail: String = "") {
        signposter.emitEvent(name, "\(detail)")
    }
}

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

    @objc private func foregrounded() {
        AppTrace.event("didBecomeActive")
        home.send(.home(action: .appForegrounded))
        AppTrace.event("foregrounded.enqueued")
    }
    @objc private func backgrounded() {
        AppTrace.event("willResignActive")
        home.send(.home(action: .appBackgrounded))
    }

    deinit {
        networkTask?.cancel()
        NotificationCenter.default.removeObserver(self)
    }
}
