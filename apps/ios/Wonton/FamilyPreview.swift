#if DEBUG
import SwiftUI
import WontonClient

/// 成员和角色页的验收夹具：启动参数带 `wonton-family-preview`（可加 `=roles`、`=join`）时生效。
/// 用核心的假数据和假 ViewModel，只为看画面，不连服务器。
@MainActor
enum FamilyPreview {
    static var variant: String {
        let argument = ProcessInfo.processInfo.arguments.first { $0.hasPrefix("wonton-family-preview") }
        return argument?.split(separator: "=").last.map(String.init) ?? "members"
    }

    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains { $0.hasPrefix("wonton-family-preview") }
    }

    static var root: some View {
        NavigationStack {
            switch variant {
            case "roles":
                RolesView(core: core, plugins: plugins)
            case "join":
                JoinInviteView(role: joinRole, invitedBy: "Lin", retry: {}, decline: {})
                    .padding(WontonTheme.spacing.pageInset)
                    .frame(maxHeight: .infinity, alignment: .top)
            default:
                MembersView(core: core, plugins: plugins)
            }
        }
    }

    private static let core: ClientCore = {
        let directory = NSTemporaryDirectory() + "family-preview"
        let core = try! ClientCore.newUiFake(dataDir: directory)
        try! core.dispatch(action: .family(action: .refreshFamily))
        return core
    }()

    private static let plugins = PluginsModel(core: core)

    private static let joinRole = JoinRoleView(
        roleId: "family",
        name: nil,
        template: .family,
        abilities: [
            JoinAbility(item: .openApps, state: .allowed),
            JoinAbility(item: .installApps, state: .needsAdmin),
            JoinAbility(item: .addFriends, state: .allowed),
            JoinAbility(item: .privateBot, state: .notAllowed),
        ]
    )
}
#endif
