import SwiftUI
import UIKit
import WontonClient

struct ContentWindowTarget: Codable, Hashable {
    let id: UUID
    let server: String
    let link: String
}

extension EnvironmentValues {
    @Entry var wontonOpenContent: ((AppLink) -> Void)?
}

struct ContentWindowButton: View {
    let target: AppLink
    @Environment(\.wontonOpenContent) private var open
    @Environment(\.supportsMultipleWindows) private var supportsMultipleWindows

    var body: some View {
        if supportsMultipleWindows, UIDevice.current.userInterfaceIdiom == .pad, let open {
            Button(.Sidebar.sidebarOpenWindow) { open(target) }
        }
    }
}

struct ContentWindowOpening: ViewModifier {
    let core: ClientCore
    @Environment(\.openWindow) private var openWindow
    @State private var problem: Problem?

    func body(content: Content) -> some View {
        content.environment(\.wontonOpenContent) { link in
            guard let server = core.servers().servers.first(where: \.current)?.id else {
                problem = Problem(title: String(localized: .Sidebar.sidebarWindowUnavailable))
                return
            }
            openWindow(value: ContentWindowTarget(id: UUID(), server: server, link: appLinkUrl(link: link)))
        }
        .wontonDialog(problem: problem, isPresented: Binding(
            get: { problem != nil }, set: { if !$0 { problem = nil } }
        ))
    }
}
