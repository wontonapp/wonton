import SwiftUI
import WontonClient

/// 邀请一个家人：填名字、选角色、要不要私人 Bot，出码后交给系统分享或当面扫。
struct MemberInviteSheet: View {
    let model: FamilyModel
    let plugins: PluginsModel
    /// 面板收起（`then` 在收起之后跑）。
    let close: (@escaping () -> Void) -> Void
    @State private var displayName = ""
    @State private var username = ""
    @State private var roleId: String?
    @State private var wantsPrivateBot = false
    @State private var submitting = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.sectionGap) {
                if let invite = model.viewModel.invite {
                    invitePanel(invite)
                } else {
                    form
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
        .onAppear(perform: pickDefaultRole)
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            field(String(localized: .Family.familyDisplayName)) {
                TextField(String(localized: .Family.familyDisplayName), text: $displayName)
                    .textInputAutocapitalization(.words)
                    .wontonField()
                    .accessibilityIdentifier("family-invite-name")
            }
            field(String(localized: .Family.familyUsername)) {
                TextField(String(localized: .Family.familyUsernameHint), text: $username)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .wontonField()
                    .accessibilityIdentifier("family-invite-username")
            }
            roleRow
            if roleAllowsPrivateBot {
                Toggle(String(localized: .Family.familyPrivateBot), isOn: $wantsPrivateBot)
                    .tint(Color(colors.primary))
                    .wontonText(WontonTheme.typography.body)
            }
            if let problem = model.viewModel.members.problem, submitting {
                ProblemRow(problem: problem.problem)
            }
            Button(String(localized: .Family.familyInviteSubmit)) { submit() }
                .buttonStyle(WontonButtonStyle())
                .disabled(!canSubmit)
                .frame(maxWidth: .infinity)
        }
    }

    private var canSubmit: Bool {
        !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !submitting
    }

    private var chosenRole: RoleDef? {
        if let roleId { return model.roles.first { $0.id == roleId } }
        return defaultRole
    }

    private var defaultRole: RoleDef? {
        guard let id = model.viewModel.roles.defaultInviteRole else { return model.roles.first }
        return model.roles.first { $0.id == id } ?? model.roles.first
    }

    private var roleAllowsPrivateBot: Bool { chosenRole?.permissions.privateBot ?? false }

    @ViewBuilder private var roleRow: some View {
        if model.access.choosesRole() {
            field(String(localized: .Family.familyRole)) {
                Menu {
                    ForEach(model.roles, id: \.id) { role in
                        Button {
                            roleId = role.id
                        } label: {
                            Text(FamilyText.role(role.name, template: role.template, id: role.id))
                        }
                    }
                } label: {
                    HStack {
                        Text(roleDisplay).foregroundStyle(Color(colors.textPrimary))
                        Spacer()
                        WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                    .wontonField()
                }
            }
        } else if let role = defaultRole {
            field(String(localized: .Family.familyRole)) {
                Text(FamilyText.role(role.name, template: role.template, id: role.id))
                    .wontonText(WontonTheme.typography.body)
                    .wontonField()
            }
        }
    }

    private var roleDisplay: String {
        chosenRole.map { FamilyText.role($0.name, template: $0.template, id: $0.id) } ?? ""
    }

    private func pickDefaultRole() {
        guard roleId == nil else { return }
        roleId = defaultRole?.id
    }

    private func submit() {
        submitting = true
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let user = username.trimmingCharacters(in: .whitespacesAndNewlines)
        model.send(.family(action: .inviteMember(
            displayName: name,
            username: user.isEmpty ? nil : user,
            privateBot: wantsPrivateBot && roleAllowsPrivateBot,
            roleId: model.access.choosesRole() ? chosenRole?.id : nil
        )))
        Task { @MainActor in
            // 等核心回话：出码、或者把问题写进成员列表的 problem。
            while submitting, model.viewModel.invite == nil, model.viewModel.members.problem == nil {
                try? await Task.sleep(for: .milliseconds(120))
            }
            submitting = false
        }
    }

    private func invitePanel(_ invite: MemberInviteView) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            Text(String(localized: .Family.familyCodeTitle(invite.displayName)))
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            if let image = QRImage.make(invite.code) {
                Image(uiImage: image)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: 240)
                    .frame(maxWidth: .infinity)
                    .accessibilityLabel(String(localized: .Family.familyQrCode))
            }
            Text(FamilyText.codeRemaining(MemberCode(expiresAtMs: invite.expiresAtMs, expired: false)))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            if let url = URL(string: invite.code) {
                ShareLink(item: url) {
                    Label(String(localized: .Family.familyCodeShare), systemImage: "square.and.arrow.up")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(String(localized: .Family.familyResendCode)) {
                    model.send(.family(action: .resendMemberCode(username: invite.username)))
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                Button(String(localized: .Family.familyRevokeCode)) {
                    close { model.send(.family(action: .dismissMemberInvite)) }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            Button(String(localized: .Common.commonDone)) {
                close { model.send(.family(action: .dismissMemberInvite)) }
            }
            .buttonStyle(WontonButtonStyle())
            .frame(maxWidth: .infinity)
        }
    }

    @ViewBuilder private func field(_ title: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Text(title)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            content()
        }
    }
}
