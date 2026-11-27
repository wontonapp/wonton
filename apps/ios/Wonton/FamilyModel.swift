import Foundation
import Observation
import SwiftUI
import WontonClient

/// 成员页的状态：核心的 `FamilyViewModel`，加核心当场拒绝的操作。
@MainActor
@Observable
final class FamilyModel {
    private(set) var viewModel: FamilyViewModel
    /// 核心拒绝上一个操作的原因，由界面弹窗展示。
    var rejection: Problem?
    private let core: ClientCore
    /// 截图测试的固定 ViewModel 不订阅核心推送。
    private let observes: Bool

    init(core: ClientCore) {
        self.core = core
        viewModel = core.family()
        observes = true
    }

    #if DEBUG
        init(core: ClientCore, viewModel: FamilyViewModel) {
            self.core = core
            self.viewModel = viewModel
            observes = false
        }
    #endif

    var access: FamilyAccess { viewModel.access }
    var members: [MemberRow] { viewModel.members.members }
    var requests: [JoinRequestRow] { viewModel.joinRequests.requests }
    var roles: [RoleDef] { viewModel.roles.roles }

    /// 进页面时取一次，之后跟着核心的推送走；页面消失时停掉轮询。
    func observe() async {
        guard observes else { return }
        send(.family(action: .refreshFamily))
        for await viewModel in core.subscribeFamily().updates() {
            self.viewModel = viewModel
        }
    }

    func leave() {
        send(.family(action: .leaveFamily))
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = error.problem
        }
    }

    /// 下拉刷新：等核心把这一轮 loading 换掉，圈才收起来。
    func refresh() async {
        send(.family(action: .refreshFamily))
        guard rejection == nil else { return }
        for await viewModel in core.subscribeFamily().updates() {
            if viewModel.members.loading || viewModel.joinRequests.loading { continue }
            return
        }
    }
}

/// 角色页的状态：只给主人，取角色和默认邀请角色。
@MainActor
@Observable
final class RolesModel {
    private(set) var viewModel: FamilyViewModel
    var rejection: Problem?
    private let core: ClientCore
    private let observes: Bool

    init(core: ClientCore) {
        self.core = core
        viewModel = core.family()
        observes = true
    }

    #if DEBUG
        init(core: ClientCore, viewModel: FamilyViewModel) {
            self.core = core
            self.viewModel = viewModel
            observes = false
        }
    #endif

    var roles: [RoleDef] { viewModel.roles.roles }
    var problem: FamilyProblem? { viewModel.roles.problem }

    func observe() async {
        guard observes else { return }
        send(.family(action: .refreshRoles))
        for await viewModel in core.subscribeFamily().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = error.problem
        }
    }
}

extension FamilyProblem {
    /// 失败原因按类别写人话；缺哪项权限点出来，核心的说明收进详情。
    var problem: Problem {
        switch reason {
        case .transportUnavailable:
            return Problem(
                title: String(localized: .Problem2.problemTransportUnavailableTitle),
                nextStep: String(localized: .Problem3.problemTransportUnavailableNextStep), detail: message)
        case .unauthorized:
            return Problem(
                title: String(localized: .Problem3.problemUnauthorizedTitle),
                nextStep: String(localized: .Problem2.problemUnauthorizedNextStep), detail: message)
        case .permissionRequired:
            let missing = missing.map { FamilyText.permission($0) }
            return Problem(
                title: String(localized: .Family.familyPermissionRequiredTitle),
                nextStep: missing.map { String(localized: .Family.familyPermissionRequiredNextStep($0)) }
                    ?? String(localized: .Problem3.problemForbiddenNextStep),
                detail: message)
        case .invalidInput:
            return Problem(title: String(localized: .Family.familyInvalidInputTitle), nextStep: message)
        case .usernameTaken:
            return Problem(
                title: String(localized: .Family.familyUsernameTakenTitle),
                nextStep: String(localized: .Family.familyUsernameTakenNextStep))
        case .notFound:
            return Problem(
                title: String(localized: .Family.familyNotFoundTitle),
                nextStep: String(localized: .Family.familyNotFoundNextStep), detail: message)
        case .removed:
            return Problem(
                title: String(localized: .Family.familyRemovedTitle),
                nextStep: String(localized: .Family.familyRemovedNextStep))
        case .deviceRegistered:
            return Problem(
                title: String(localized: .Family.familyDeviceRegisteredTitle),
                nextStep: String(localized: .Family.familyDeviceRegisteredNextStep))
        case .notReady:
            return Problem(
                title: String(localized: .Problem0.problemNotReadyTitle),
                nextStep: String(localized: .Family.familyNotReadyNextStep))
        case .matrixUnavailable:
            return Problem(
                title: String(localized: .Family.familyMatrixUnavailableTitle),
                nextStep: String(localized: .Family.familyMatrixUnavailableNextStep), detail: message)
        case .storage:
            return Problem(
                title: String(localized: .Problem3.problemStorageTitle),
                nextStep: String(localized: .Problem1.problemStorageNextStep), detail: message)
        case .upstream:
            return Problem(
                title: String(localized: .Problem1.problemGenericTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep), detail: message)
        case .protocol:
            return Problem(title: String(localized: .Family.familyProtocolTitle), nextStep: String(localized: .Family.familyProtocolNextStep))
        case .other:
            return Problem(
                title: String(localized: .Problem1.problemGenericTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep), detail: message)
        }
    }
}

enum FamilyText {
    /// 角色的显示名：主人起过名用名字，模板没改名就按模板本地化，都没有用 ID。
    static func role(_ name: String?, template: RoleTemplate?, id: String) -> String {
        if let name, !name.isEmpty { return name }
        return template.map(templateName) ?? id
    }

    static func templateName(_ template: RoleTemplate) -> String {
        switch template {
        case .admin: String(localized: .Family.familyRoleTemplateAdmin)
        case .family: String(localized: .Family.familyRoleTemplateFamily)
        case .child: String(localized: .Family.familyRoleTemplateChild)
        case .friend: String(localized: .Family.familyRoleTemplateFriend)
        case .other: String(localized: .Family.familyRoleTemplateOther)
        }
    }

    static func status(_ status: MemberStatus) -> LocalizedStringResource {
        switch status {
        case .invited: .Family.familyStatusInvited
        case .joined: .Family.familyStatusJoined
        case .removed, .other: .Family.familyStatusOther
        }
    }

    /// 一项权限的人话（ADR-0088 的表格顺序），角色页和加入页共用。
    static func permission(_ item: PermissionItem) -> String {
        switch item {
        case .openApps: String(localized: .Family.familyPermissionOpenApps)
        case .pluginAdmin: String(localized: .Family.familyPermissionPluginAdmin)
        case .installApps: String(localized: .Family.familyPermissionInstallApps)
        case .modelSettings: String(localized: .Family.familyPermissionModelSettings)
        case .serverSettings: String(localized: .Family.familyPermissionServerSettings)
        case .manageMembers: String(localized: .Family.familyPermissionManageMembers)
        case .addFriends: String(localized: .Family.familyPermissionAddFriends)
        case .shareApps: String(localized: .Family.familyPermissionShareApps)
        case .ai: String(localized: .Family.familyPermissionAi)
        case .privilegedTools: String(localized: .Family.familyPermissionPrivilegedTools)
        case .privateBot: String(localized: .Family.familyPermissionPrivateBot)
        case .sharedBots: String(localized: .Family.familyPermissionSharedBots)
        case .lendOwnBot: String(localized: .Family.familyPermissionLendOwnBot)
        case .owner: String(localized: .Family.familyPermissionOwner)
        case .other: String(localized: .Family.familyPermissionOther)
        }
    }

    /// 邀请码还剩多久：不到一天时按小时和分钟，过了就说过期。
    static func codeRemaining(_ code: MemberCode) -> String {
        guard !code.expired else { return String(localized: .Family.familyCodeExpired) }
        let expires = Date(timeIntervalSince1970: Double(code.expiresAtMs) / 1000)
        let remaining = max(0, expires.timeIntervalSinceNow)
        let duration = Duration.seconds(Int(remaining.rounded()))
        return String(localized: .Family.familyCodeExpires(duration.formatted(.units(allowed: [.hours, .minutes], width: .abbreviated))))
    }
}

extension View {
    /// 令牌里常用的两句：条目标题、说明文字。
    func wontonTextItemTitle() -> some View { wontonText(WontonTheme.typography.itemTitle) }
    func wontonTextSupporting() -> some View { wontonText(WontonTheme.typography.supporting) }
}
