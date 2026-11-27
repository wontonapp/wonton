import SwiftUI
import WontonClient

/// 新建或修改一个角色：名字、逐项权限、从模板起步。模板只提供起点，保存的就是勾选后的结果。
struct RoleEditorView: View {
    let model: RolesModel
    let plugins: PluginsModel
    let role: RoleDef?
    let close: (@escaping () -> Void) -> Void

    @State private var name: String
    @State private var openAppsAll: Bool
    @State private var openAppsSelected: Set<String>
    @State private var pluginAdminMode: AdminMode
    @State private var pluginAdminSelected: Set<String>
    @State private var installApps: GateChoice
    @State private var modelSettings: Bool
    @State private var serverSettings: Bool
    @State private var manageMembers: Bool
    @State private var addFriends: GateChoice
    @State private var shareApps: Bool
    @State private var ai: AiChoice
    @State private var privilegedTools: Bool
    @State private var privateBot: Bool
    @State private var sharedBots: Bool
    @State private var lendOwnBot: Bool

    @State private var showingDelete: Bool
    @Environment(\.wontonColors) private var colors

    init(model: RolesModel, plugins: PluginsModel, role: RoleDef?, close: @escaping (@escaping () -> Void) -> Void) {
        self.model = model
        self.plugins = plugins
        self.role = role
        self.close = close
        _name = State(initialValue: role.map { FamilyText.role($0.name, template: $0.template, id: $0.id) } ?? "")
        let permissions = role?.permissions
        _openAppsAll = State(initialValue: permissions?.openApps == .all)
        switch permissions?.openApps {
        case let .selected(pluginIds)?: _openAppsSelected = State(initialValue: Set(pluginIds))
        default: _openAppsSelected = State(initialValue: [])
        }
        let adminMode: AdminMode
        let adminSelected: Set<String>
        switch permissions?.pluginAdmin {
        case .allOpenable?:
            adminMode = .allOpenable
            adminSelected = []
        case let .selected(pluginIds)?:
            adminMode = .selected
            adminSelected = Set(pluginIds)
        default:
            adminMode = .noApps
            adminSelected = []
        }
        _pluginAdminMode = State(initialValue: adminMode)
        _pluginAdminSelected = State(initialValue: adminSelected)
        _installApps = State(initialValue: GateChoice(permissions?.installApps ?? .deny))
        _modelSettings = State(initialValue: permissions?.modelSettings ?? false)
        _serverSettings = State(initialValue: permissions?.serverSettings ?? false)
        _manageMembers = State(initialValue: permissions?.manageMembers ?? false)
        _addFriends = State(initialValue: GateChoice(permissions?.addFriends ?? .deny))
        _shareApps = State(initialValue: permissions?.shareApps ?? false)
        _ai = State(initialValue: AiChoice(permissions?.ai ?? .readOnly))
        _privilegedTools = State(initialValue: permissions?.privilegedTools ?? false)
        _privateBot = State(initialValue: permissions?.privateBot ?? false)
        _sharedBots = State(initialValue: permissions?.sharedBots ?? false)
        _lendOwnBot = State(initialValue: permissions?.lendOwnBot ?? false)
        _showingDelete = State(initialValue: false)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                nameField
                if role == nil { templates }
                permissionsSection
                if let role, role.people == 0, model.viewModel.roles.defaultInviteRole != role.id {
                    deleteButton(role)
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    /// 删掉没人用的角色；默认角色和在用的角色删不掉，由核心回问题。
    @ViewBuilder private func deleteButton(_ role: RoleDef) -> some View {
        Button {
            showingDelete = true
        } label: {
            Text(.Family.familyRoleDelete)
                .frame(maxWidth: .infinity)
        }
        .buttonStyle(WontonButtonStyle(kind: .secondary))
        .wontonDialog(
            String(localized: .Family.familyRoleDeleteTitle(FamilyText.role(role.name, template: role.template, id: role.id))),
            message: String(localized: .Family.familyRoleDeleteMessage),
            isPresented: $showingDelete,
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)),
                WontonDialogAction(title: String(localized: .Family.familyRoleDelete), kind: .danger) {
                    model.send(.family(action: .deleteRole(roleId: role.id)))
                    close {}
                },
            ]
        )
    }
}
extension RoleEditorView {
    @ViewBuilder private var nameField: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Text(.Family.familyRoleName)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            TextField(String(localized: .Family.familyRoleName), text: $name)
                .textInputAutocapitalization(.words)
                .wontonField()
                .accessibilityIdentifier("family-role-name")
        }
    }

    /// 新建时从模板起步：把模板角色现在的勾选拷过来，再改。
    @ViewBuilder private var templates: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.Family.familyRoleStartFrom)
                .wontonTextSupporting()
            ForEachTemplate(roles: model.roles) { template, source in
                Button {
                    apply(source)
                } label: {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        WontonIcon.list.view(size: WontonTheme.size.iconSmall)
                            .foregroundStyle(Color(colors.textSecondary))
                        Text(FamilyText.templateName(template))
                            .wontonText(WontonTheme.typography.body)
                        Spacer()
                    }
                    .padding(WontonTheme.spacing.s12)
                    .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func apply(_ source: RoleDef) {
        name = FamilyText.role(source.name, template: source.template, id: source.id)
        let p = source.permissions
        openAppsAll = p.openApps == .all
        if case let .selected(pluginIds) = p.openApps { openAppsSelected = Set(pluginIds) }
        switch p.pluginAdmin {
        case .allOpenable: pluginAdminMode = .allOpenable
        case let .selected(pluginIds):
            pluginAdminMode = .selected
            pluginAdminSelected = Set(pluginIds)
        case .noApps, .other: pluginAdminMode = .noApps
        }
        installApps = GateChoice(p.installApps)
        modelSettings = p.modelSettings
        serverSettings = p.serverSettings
        manageMembers = p.manageMembers
        addFriends = GateChoice(p.addFriends)
        shareApps = p.shareApps
        ai = AiChoice(p.ai)
        privilegedTools = p.privilegedTools
        privateBot = p.privateBot
        sharedBots = p.sharedBots
        lendOwnBot = p.lendOwnBot
    }

    @ViewBuilder private var permissionsSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            PermissionGroup(title: .Family.familyPermissionGroupApps) {
                scopeRow(
                    title: .Family.familyPermissionOpenApps,
                    all: $openAppsAll,
                    selected: $openAppsSelected
                )
                separator
                adminRow
                separator
                gateRow(title: .Family.familyPermissionInstallApps, choice: $installApps)
            }
            PermissionGroup(title: .Family.familyPermissionGroupServer) {
                boolRow(.Family.familyPermissionModelSettings, $modelSettings)
                separator
                boolRow(.Family.familyPermissionServerSettings, $serverSettings)
                separator
                boolRow(.Family.familyPermissionManageMembers, $manageMembers)
                separator
                gateRow(title: .Family.familyPermissionAddFriends, choice: $addFriends)
                separator
                boolRow(.Family.familyPermissionShareApps, $shareApps)
            }
            PermissionGroup(title: .Family.familyPermissionGroupAi) {
                aiRow
                separator
                boolRow(.Family.familyPermissionPrivilegedTools, $privilegedTools)
                separator
                boolRow(.Family.familyPermissionPrivateBot, $privateBot)
                separator
                boolRow(.Family.familyPermissionSharedBots, $sharedBots)
                separator
                boolRow(.Family.familyPermissionLendOwnBot, $lendOwnBot)
            }
            if let roleError = model.viewModel.roles.problem {
                ProblemRow(problem: roleError.problem)
                    .padding(WontonTheme.spacing.s16)
            }
            Button(String(localized: .Family.familyRoleSave)) { save() }
                .buttonStyle(WontonButtonStyle())
                .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .frame(maxWidth: .infinity)
                .padding(WontonTheme.spacing.s16)
        }
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
    }

    private var separator: some View { WontonSeparator() }

    @ViewBuilder private func boolRow(_ title: LocalizedStringResource, _ value: Binding<Bool>) -> some View {
        Toggle(isOn: value) {
            Text(title)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
        }
        .tint(Color(colors.primary))
        .padding(WontonTheme.spacing.s16)
    }

    @ViewBuilder private func gateRow(title: LocalizedStringResource, choice: Binding<GateChoice>) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(title)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
            Picker(selection: choice) {
                ForEach(GateChoice.allCases) { value in
                    Text(value.label).tag(value)
                }
            } label: { EmptyView() }
            .pickerStyle(.segmented)
        }
        .padding(WontonTheme.spacing.s16)
    }

    @ViewBuilder private var aiRow: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.Family.familyPermissionAi)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
            Picker(selection: $ai) {
                ForEach(AiChoice.allCases) { value in
                    Text(value.label).tag(value)
                }
            } label: { EmptyView() }
            .pickerStyle(.segmented)
        }
        .padding(WontonTheme.spacing.s16)
    }

    /// 能打开哪些应用：全部，或勾选几个。
    @ViewBuilder private func scopeRow(
        title: LocalizedStringResource,
        all: Binding<Bool>,
        selected: Binding<Set<String>>
    ) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Toggle(isOn: all) {
                Text(title)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
            }
            .tint(Color(colors.primary))
            if !all.wrappedValue {
                appToggles(selected)
            }
        }
        .padding(WontonTheme.spacing.s16)
    }

    @ViewBuilder private var adminRow: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.Family.familyPermissionPluginAdmin)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
            Picker(selection: $pluginAdminMode) {
                ForEach(AdminMode.allCases) { value in
                    Text(value.label).tag(value)
                }
            } label: { EmptyView() }
            .pickerStyle(.segmented)
            if pluginAdminMode == .selected {
                appToggles($pluginAdminSelected)
            }
        }
        .padding(WontonTheme.spacing.s16)
    }

    @ViewBuilder private func appToggles(_ selection: Binding<Set<String>>) -> some View {
        let installed = plugins.viewModel.installed?.plugins ?? []
        if installed.isEmpty {
            Text(.Family.familyRoleNoAppsInstalled)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        } else {
            VStack(spacing: 0) {
                ForEach(installed, id: \.id) { plugin in
                    Toggle(isOn: Binding(
                        get: { selection.wrappedValue.contains(plugin.id) },
                        set: { on in
                            if on { selection.wrappedValue.insert(plugin.id) }
                            else { selection.wrappedValue.remove(plugin.id) }
                        }
                    )) {
                        Text(plugin.title)
                            .wontonText(WontonTheme.typography.body)
                    }
                    .tint(Color(colors.primary))
                }
            }
        }
    }

    private func save() {
        let draft = RoleDraft(
            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
            permissions: buildPermissions()
        )
        if let role {
            model.send(.family(action: .updateRole(roleId: role.id, draft: draft)))
        } else {
            model.send(.family(action: .createRole(draft: draft)))
        }
        close {}
    }

    private func buildPermissions() -> Permissions {
        let pluginAdmin: AdminScope = switch pluginAdminMode {
        case .allOpenable: .allOpenable
        case .selected: .selected(pluginIds: Array(pluginAdminSelected))
        case .noApps: .noApps
        }
        return Permissions(
            openApps: openAppsAll ? .all : .selected(pluginIds: Array(openAppsSelected)),
            pluginAdmin: pluginAdmin,
            installApps: installApps.gate,
            modelSettings: modelSettings,
            serverSettings: serverSettings,
            manageMembers: manageMembers,
            addFriends: addFriends.gate,
            shareApps: shareApps,
            ai: ai.level,
            privilegedTools: privilegedTools,
            privateBot: privateBot,
            sharedBots: sharedBots,
            lendOwnBot: lendOwnBot
        )
    }
}

private struct PermissionGroup<Content: View>: View {
    let title: LocalizedStringResource
    @ViewBuilder let content: Content
    @Environment(\.wontonColors) private var colors

    init(title: LocalizedStringResource, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.top, WontonTheme.spacing.s12)
            content
        }
    }
}

private struct ForEachTemplate<Content: View>: View {
    let roles: [RoleDef]
    let content: (RoleTemplate, RoleDef) -> Content

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            ForEach(Array(Self.order.enumerated()), id: \.offset) { _, template in
                if let source = roles.first(where: { $0.template == template }) {
                    content(template, source)
                }
            }
        }
    }

    private static var order: [RoleTemplate] { [.admin, .family, .child, .friend] }
}

enum GateChoice: String, CaseIterable, Identifiable {
    case allow, askAdmin, deny

    init(_ gate: Gate) {
        self = switch gate {
        case .allow: .allow
        case .askAdmin: .askAdmin
        default: .deny
        }
    }

    var id: String { rawValue }
    var label: LocalizedStringResource {
        switch self {
        case .allow: .Family.familyGateAllow
        case .askAdmin: .Family.familyGateAskAdmin
        case .deny: .Family.familyGateDeny
        }
    }
    var gate: Gate {
        switch self {
        case .allow: .allow
        case .askAdmin: .askAdmin
        case .deny: .deny
        }
    }
}

enum AdminMode: String, CaseIterable, Identifiable {
    case allOpenable, selected, noApps

    var id: String { rawValue }
    var label: LocalizedStringResource {
        switch self {
        case .allOpenable: .Family.familyAdminAll
        case .selected: .Family.familyAdminSelected
        case .noApps: .Family.familyAdminNone
        }
    }
}

enum AiChoice: String, CaseIterable, Identifiable {
    case readOnly, adminApprovesDeletes, selfConfirms

    init(_ level: AiLevel) {
        self = switch level {
        case .readOnly, .other: .readOnly
        case .adminApprovesDeletes: .adminApprovesDeletes
        case .selfConfirms: .selfConfirms
        }
    }

    var id: String { rawValue }
    var label: LocalizedStringResource {
        switch self {
        case .readOnly: .Family.familyAiReadOnly
        case .adminApprovesDeletes: .Family.familyAiAskAdmin
        case .selfConfirms: .Family.familyAiSelf
        }
    }
    var level: AiLevel {
        switch self {
        case .readOnly: .readOnly
        case .adminApprovesDeletes: .adminApprovesDeletes
        case .selfConfirms: .selfConfirms
        }
    }
}
