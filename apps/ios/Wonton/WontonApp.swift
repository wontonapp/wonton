import SwiftUI
import WontonClient

@main
struct WontonApp: App {
    /// 整个 App 只有一个核心实例，状态都在它里面。
    @State private var sessionList = SessionListModel(core: ClientCore())

    var body: some Scene {
        WindowGroup {
            SessionListView(model: sessionList)
        }
    }
}
