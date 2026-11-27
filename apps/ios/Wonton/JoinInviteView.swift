import SwiftUI
import WontonClient

/// 被邀请人的加入页（ADR-0088）：说清给什么角色、这个角色能做什么，再决定加入还是算了。
/// 码交上去以后请求已经在管理员那里等批；这里只负责说清楚，并隔几秒自动再看一眼。
struct JoinInviteView: View {
    let role: JoinRoleView?
    let invitedBy: String?
    /// 用同一个码再交一次：管理员同意后会变成已配对。
    let retry: (() -> Void)?
    let decline: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                Text(.Family.familyJoinTitle)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                Text(.Family.familyJoinBlurb)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                if let invitedBy {
                    Text(.Family.familyJoinInvitedBy(invitedBy))
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            if let role {
                WontonGroup {
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                        if let name = role.name, !name.isEmpty {
                            HStack(spacing: WontonTheme.spacing.s8) {
                                Text(name).wontonTextItemTitle()
                                if let template = role.template {
                                    Tag(text: FamilyText.templateName(template), tint: colors.textSecondary)
                                }
                            }
                        } else {
                            Text(FamilyText.templateName(role.template ?? .other)).wontonTextItemTitle()
                        }
                        if !role.abilities.isEmpty {
                            Text(.Family.familyJoinAbilities)
                                .wontonText(WontonTheme.typography.supporting)
                                .foregroundStyle(Color(colors.textSecondary))
                            ForEach(Array(role.abilities.enumerated()), id: \.offset) { _, ability in
                                HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                                    abilityIcon(ability.state)
                                        .view(size: WontonTheme.size.iconSmall)
                                        .foregroundStyle(Color(colors.textSecondary))
                                    Text(abilityLine(ability))
                                        .wontonText(WontonTheme.typography.body)
                                        .foregroundStyle(Color(colors.textPrimary))
                                }
                            }
                        }
                    }
                    .padding(WontonTheme.spacing.s16)
                }
            }
            Text(.Family.familyJoinPrivacy)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            HStack(spacing: WontonTheme.spacing.s8) {
                if let retry {
                    Button(.Family.familyJoinAccept, action: retry)
                        .buttonStyle(WontonButtonStyle())
                        .accessibilityIdentifier("family-join-accept")
                }
                Button(.Family.familyJoinDecline, action: decline)
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                    .accessibilityIdentifier("family-join-decline")
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                ProgressView()
                Text(.Family.familyJoinWaiting)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            .frame(maxWidth: .infinity, alignment: .center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("family-join-page")
        .task(id: retry != nil) {
            guard retry != nil else { return }
            // 管理员同意后同一个码再交会直接配上；几秒看一次，别让被邀请人守着点。
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(4))
                guard !Task.isCancelled else { return }
                retry?()
            }
        }
    }

    private func abilityIcon(_ state: JoinAbilityState) -> WontonIcon {
        switch state {
        case .allowed: .checkCircle
        case .needsAdmin: .clock
        case .notAllowed, .other: .lock
        }
    }

    private func abilityLine(_ ability: JoinAbility) -> String {
        let name = FamilyText.permission(ability.item)
        switch ability.state {
        case .allowed: return name
        case .needsAdmin: return "\(name) · \(String(localized: .Family.familyGateAskAdmin))"
        case .notAllowed: return "\(name) · \(String(localized: .Family.familyGateDeny))"
        case .other: return name
        }
    }
}
