import SwiftUI
import WontonClient

/// 给一个成员换角色：列出全部角色，当前那个打勾；选中即发回核心。
struct MemberRolePicker: View {
    let model: FamilyModel
    let member: MemberRow
    let close: (@escaping () -> Void) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                Text(String(localized: .Family.familyChangeRoleFor(member.displayName)))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                ForEach(model.roles, id: \.id) { role in
                    Button {
                        model.send(.family(action: .assignMemberRole(username: member.username, roleId: role.id)))
                        close {}
                    } label: {
                        HStack(spacing: WontonTheme.spacing.s12) {
                            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                                Text(FamilyText.role(role.name, template: role.template, id: role.id))
                                    .wontonText(WontonTheme.typography.body)
                                    .foregroundStyle(Color(colors.textPrimary))
                                Text(String(localized: .Family.familyPeopleCount(Int(role.people))))
                                    .wontonText(WontonTheme.typography.caption)
                                    .foregroundStyle(Color(colors.textSecondary))
                            }
                            Spacer(minLength: 0)
                            if role.id == member.roleId {
                                WontonIcon.check.view(size: WontonTheme.size.iconSmall)
                                    .foregroundStyle(Color(colors.primary))
                            }
                        }
                        .padding(WontonTheme.spacing.s12)
                        .background(
                            RoundedRectangle(cornerRadius: WontonTheme.radii.control)
                                .fill(Color(role.id == member.roleId ? colors.accentFill : colors.surfaceSubtle))
                        )
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }
}

/// 移除一个成员：吊销设备、停用账号；私人空间删不删、Bot 数据留不留由主人定。
struct MemberRemoveSheet: View {
    let model: FamilyModel
    let member: MemberRow
    let close: (@escaping () -> Void) -> Void
    @State private var deleteHome = true
    @State private var keepBotData = true
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                Text(String(localized: .Family.familyRemoveBlurb(member.displayName)))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                VStack(spacing: WontonTheme.spacing.s12) {
                    Toggle(isOn: $deleteHome) {
                        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                            Text(String(localized: .Family.familyRemoveDeleteHome))
                                .wontonText(WontonTheme.typography.body)
                            Text(String(localized: .Family.familyRemoveDeleteHomeHint))
                                .wontonText(WontonTheme.typography.caption)
                                .foregroundStyle(Color(colors.textSecondary))
                        }
                    }
                    .tint(Color(colors.primary))
                    Toggle(isOn: $keepBotData) {
                        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                            Text(String(localized: .Family.familyRemoveKeepBotData))
                                .wontonText(WontonTheme.typography.body)
                            Text(String(localized: .Family.familyRemoveKeepBotDataHint))
                                .wontonText(WontonTheme.typography.caption)
                                .foregroundStyle(Color(colors.textSecondary))
                        }
                    }
                    .tint(Color(colors.primary))
                }
                Button(String(localized: .Family.familyRemoveMember)) {
                    model.send(.family(action: .removeMember(
                        username: member.username,
                        deleteHome: deleteHome,
                        keepBotData: keepBotData
                    )))
                    close {}
                }
                .buttonStyle(WontonButtonStyle(kind: .danger))
                .frame(maxWidth: .infinity)
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }
}
