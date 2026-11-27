import SwiftUI
import WontonClient

/// 角色页（ADR-0088）：只有主人能进。角色是逐项勾选的权限，内置四个模板；这里管名单和默认角色。
struct RolesView: View {
    @State private var model: RolesModel
    let plugins: PluginsModel
    @State private var editing: RoleDef?
    @State private var creating = false
    @Environment(\.wontonColors) private var colors

    init(core: ClientCore, plugins: PluginsModel) {
        _model = State(initialValue: RolesModel(core: core))
        self.plugins = plugins
    }

    #if DEBUG
        init(core: ClientCore, viewModel: FamilyViewModel, plugins: PluginsModel) {
            _model = State(initialValue: RolesModel(core: core, viewModel: viewModel))
            self.plugins = plugins
        }
    #endif

    var body: some View {
        content
            .wontonPushedScreen(String(localized: .Family.familyRolesTitle), usesSystemToolbar: true) {
                WontonIconButton(icon: .add, label: String(localized: .Family.familyRoleNew)) {
                    creating = true
                }
            }
            .task { await model.observe() }
            .refreshable { model.send(.family(action: .refreshRoles)) }
            .wontonSheet(
                isPresented: Binding(
                    get: { editing != nil },
                    set: { if !$0 { editing = nil } }
                ),
                title: editing.map { FamilyText.role($0.name, template: $0.template, id: $0.id) },
                usesSystemPresentation: true
            ) { close in
                if let role = editing {
                    RoleEditorView(model: model, plugins: plugins, role: role, close: close)
                }
            }
            .wontonSheet(
                isPresented: $creating,
                title: String(localized: .Family.familyRoleNew),
                usesSystemPresentation: true
            ) { close in
                RoleEditorView(model: model, plugins: plugins, role: nil, close: close)
            }
            .wontonDialog(
                problem: model.rejection,
                isPresented: Binding(
                    get: { model.rejection != nil },
                    set: { if !$0 { model.rejection = nil } }
                )
            )
    }

    @ViewBuilder private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem = model.problem {
                    ProblemRow(problem: problem.problem) { model.send(.family(action: .refreshRoles)) }
                }
                rolesSection
                defaultSection
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    private var rolesSection: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            SectionHint(.Family.familyRoleList)
            if model.roles.isEmpty {
                Text(.Family.familyRoleEmpty)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(model.roles.enumerated()), id: \.element.id) { index, role in
                        if index > 0 { WontonSeparator() }
                        Button { editing = role } label: {
                            RoleRowView(role: role, plugins: plugins, isDefault: defaultRoleID == role.id)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
            }
        }
    }

    private var defaultSection: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            SectionHint(.Family.familyDefaultRole)
            VStack(spacing: 0) {
                ForEach(model.roles, id: \.id) { role in
                    Button {
                        guard role.id != defaultRoleID else { return }
                        model.send(.family(action: .setDefaultRole(roleId: role.id)))
                    } label: {
                        HStack(spacing: WontonTheme.spacing.s12) {
                            Text(FamilyText.role(role.name, template: role.template, id: role.id))
                                .wontonText(WontonTheme.typography.body)
                                .foregroundStyle(Color(colors.textPrimary))
                            Spacer()
                            if role.id == defaultRoleID {
                                WontonIcon.check.view(size: WontonTheme.size.iconSmall)
                                    .foregroundStyle(Color(colors.primary))
                            }
                        }
                        .padding(.horizontal, WontonTheme.spacing.s16)
                        .frame(minHeight: WontonTheme.size.target)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
        }
    }

    private var defaultRoleID: String? { model.viewModel.roles.defaultInviteRole }
}

private struct SectionHint: View {
    let text: LocalizedStringResource
    @Environment(\.wontonColors) private var colors

    init(_ text: LocalizedStringResource) { self.text = text }

    var body: some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
    }
}

/// 角色列表的一行：名字、来源模板、人数、默认标记。
private struct RoleRowView: View {
    let role: RoleDef
    let plugins: PluginsModel
    let isDefault: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    Text(FamilyText.role(role.name, template: role.template, id: role.id))
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    if isDefault {
                        Tag(text: String(localized: .Family.familyRoleDefaultTag), tint: colors.primary)
                    }
                }
                Text(String(localized: .Family.familyRolePeople(Int(role.people))))
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            Spacer(minLength: 0)
            WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .contentShape(Rectangle())
    }
}
