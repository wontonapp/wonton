import SwiftUI
import WontonClient

/// 设置里的“成员”（ADR-0088）：主人和管理员在这里领人进门、批加入请求、移除丢了的手机。
/// 状态和动作都在客户端核心，这里只渲染 ViewModel、把操作发回核心。
struct MembersView: View {
    @State private var model: FamilyModel
    let plugins: PluginsModel
    @State private var showsInvite = false
    @State private var actionMember: MemberRow?
    @State private var removing: MemberRow?
    @State private var assigning: MemberRow?
    @Environment(\.wontonColors) private var colors

    init(core: ClientCore, plugins: PluginsModel) {
        _model = State(initialValue: FamilyModel(core: core))
        self.plugins = plugins
    }

    #if DEBUG
        /// 截图测试拿固定 ViewModel 画界面，不订阅核心推送。
        init(core: ClientCore, viewModel: FamilyViewModel, plugins: PluginsModel) {
            _model = State(initialValue: FamilyModel(core: core, viewModel: viewModel))
            self.plugins = plugins
        }
    #endif

    var body: some View {
        content
            .wontonPushedScreen(String(localized: .Family.familyMembersTitle), usesSystemToolbar: true) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    if model.access.managesRoles() {
                        WontonIconLink(icon: .person, label: String(localized: .Family.familyRolesTitle), value: SettingsRoute.familyRoles)
                    }
                    if model.access.managesMembers() {
                        WontonIconButton(icon: .add, label: String(localized: .Family.familyInvite)) {
                            showsInvite = true
                        }
                    }
                }
            }
            .task { await model.observe() }
            .onDisappear { model.leave() }
            .refreshable { await model.refresh() }
            .wontonSheet(
                isPresented: $showsInvite,
                title: String(localized: .Family.familyInviteTitle),
                usesSystemPresentation: true
            ) { close in
                MemberInviteSheet(model: model, plugins: plugins, close: close)
            }
            .wontonSheet(
                isPresented: Binding(
                    get: { assigning != nil },
                    set: { if !$0 { assigning = nil } }
                ),
                title: assigning.map { String(localized: .Family.familyChangeRoleFor($0.displayName)) },
                usesSystemPresentation: true
            ) { close in
                if let member = assigning {
                    MemberRolePicker(model: model, member: member, close: close)
                }
            }
            .wontonSheet(
                isPresented: Binding(
                    get: { removing != nil },
                    set: { if !$0 { removing = nil } }
                ),
                title: removing.map { String(localized: .Family.familyRemoveTitle($0.displayName)) },
                usesSystemPresentation: true
            ) { close in
                if let member = removing {
                    MemberRemoveSheet(model: model, member: member, close: close)
                }
            }
            .wontonActionSheet(
                isPresented: Binding(
                    get: { actionMember != nil },
                    set: { if !$0 { actionMember = nil } }
                ),
                title: actionMember?.displayName,
                actions: actionMember.map(memberActions) ?? []
            )
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
                identity
                if let problem = model.viewModel.members.problem {
                    ProblemRow(problem: problem.problem) { Task { await model.refresh() } }
                }
                if model.access.managesMembers() {
                    requestsSection
                }
                membersSection
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    /// 这一页是给谁看的：主人不受角色限制；管理员写自己的角色。
    private var identity: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Text(model.access.isOwner ? String(localized: .Family.familyYouAreOwner) : String(localized: .Family.familyYouAreManager))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            if let role = model.access.roleName, !model.access.isOwner {
                Text(role)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
            }
        }
    }

    /// 还没处理的加入请求，最新的在前；同意后设备登记在这个人名下。
    @ViewBuilder private var requestsSection: some View {
        if !model.requests.isEmpty || model.viewModel.joinRequests.loading || model.viewModel.joinRequests.problem != nil {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                SectionTitle(String(localized: .Family.familyRequestsSection))
                if let problem = model.viewModel.joinRequests.problem {
                    ProblemRow(problem: problem.problem) {
                        model.send(.family(action: .refreshJoinRequests))
                    }
                }
                ForEach(model.requests, id: \.id) { request in
                    JoinRequestCard(
                        request: request,
                        approve: { model.send(.family(action: .approveJoinRequest(id: request.id))) },
                        reject: { model.send(.family(action: .rejectJoinRequest(id: request.id))) }
                    )
                }
            }
        }
    }

    /// 还在的人；已移除的不再列出。每行点开是这一个成员的操作。
    private var membersSection: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            SectionTitle(String(localized: .Family.familyMembersSection))
            if model.members.isEmpty, model.viewModel.members.loading {
                HStack(spacing: WontonTheme.spacing.s12) {
                    ProgressView()
                    Text(.Family.familyEmpty)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            } else if model.members.isEmpty {
                Text(.Family.familyEmpty)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(model.members.enumerated()), id: \.element.username) { index, member in
                        if index > 0 { WontonSeparator() }
                        Button { actionMember = member } label: {
                            MemberRowView(member: member)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
            }
        }
    }

    /// 一个成员能做的事：有码时重发/作废，主人还能换角色、移除。
    private func memberActions(_ member: MemberRow) -> [WontonSheetAction] {
        var actions: [WontonSheetAction] = []
        if member.status == .invited {
            if member.code != nil {
                actions.append(WontonSheetAction(title: String(localized: .Family.familyResendCode), icon: .refresh) {
                    model.send(.family(action: .resendMemberCode(username: member.username)))
                })
                actions.append(WontonSheetAction(title: String(localized: .Family.familyRevokeCode), icon: .trash) {
                    model.send(.family(action: .revokeMemberCode(username: member.username)))
                })
            } else {
                actions.append(WontonSheetAction(title: String(localized: .Family.familySendCode), icon: .send) {
                    model.send(.family(action: .resendMemberCode(username: member.username)))
                })
            }
        }
        if model.access.managesRoles() {
            actions.append(WontonSheetAction(title: String(localized: .Family.familyChangeRole), icon: .person) {
                assigning = member
            })
        }
        actions.append(WontonSheetAction(title: String(localized: .Family.familyRemoveMember), icon: .trash, isDanger: true) {
            removing = member
        })
        return actions
    }
}

private struct SectionTitle: View {
    let title: String
    @Environment(\.wontonColors) private var colors

    init(_ title: String) { self.title = title }

    var body: some View {
        Text(title)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
    }
}

private struct MemberRowView: View {
    let member: MemberRow
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .center, spacing: WontonTheme.spacing.s12) {
            MemberInitial(name: member.displayName)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    Text(member.displayName)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    Tag(text: FamilyText.role(member.roleName, template: member.roleTemplate, id: member.roleId),
                        tint: colors.textSecondary)
                }
                Text(verbatim: member.username)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                statusLine
            }
            Spacer(minLength: 0)
            WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder private var statusLine: some View {
        switch member.status {
        case .invited:
            if let code = member.code {
                Text(FamilyText.codeRemaining(code))
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(code.expired ? colors.danger : colors.textSecondary))
            }
        case .joined:
            Text(.Family.familyStatusJoined)
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
        case .removed, .other:
            Text(.Family.familyStatusOther)
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
        }
    }
}

private struct MemberInitial: View {
    let name: String
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(name.isEmpty ? "?" : String(name.prefix(1)).uppercased())
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(Color(colors.textSecondary))
            .frame(width: 40, height: 40)
            .background(Circle().fill(Color(colors.surfaceSubtle)))
    }
}

private struct JoinRequestCard: View {
    let request: JoinRequestRow
    let approve: () -> Void
    let reject: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s8) {
                MemberInitial(name: request.displayName)
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        Text(request.displayName).wontonText(WontonTheme.typography.body)
                        Tag(text: FamilyText.role(request.roleName, template: request.roleTemplate, id: request.roleId), tint: colors.textSecondary)
                    }
                    Text(request.deviceName).wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                Spacer(minLength: 0)
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(.Family.familyApprove) { approve() }.buttonStyle(WontonSmallButtonStyle())
                Button(.Family.familyReject) { reject() }.buttonStyle(WontonSmallButtonStyle(kind: .secondary))
            }
        }
        .padding(WontonTheme.spacing.s12)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
    }
}
