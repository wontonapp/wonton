package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.FamilyFailure
import app.wonton.protocol.FamilyProblem
import app.wonton.protocol.JoinAbility
import app.wonton.protocol.JoinAbilityState
import app.wonton.protocol.MemberCode
import app.wonton.protocol.MemberStatus
import app.wonton.protocol.PermissionItem
import app.wonton.protocol.RoleTemplate
import java.time.Duration
import java.time.Instant

/** 角色的显示名：起过名用名字，模板没改名按模板本地化，都没有用 ID。 */
@Composable
fun roleName(name: String?, template: RoleTemplate?, id: String): String {
    if (!name.isNullOrEmpty()) return name
    return template?.let { templateName(it) } ?: id
}

@Composable
fun templateName(template: RoleTemplate): String = stringResource(
    when (template) {
        RoleTemplate.ADMIN -> R.string.family_roleTemplateAdmin
        RoleTemplate.FAMILY -> R.string.family_roleTemplateFamily
        RoleTemplate.CHILD -> R.string.family_roleTemplateChild
        RoleTemplate.FRIEND -> R.string.family_roleTemplateFriend
        RoleTemplate.OTHER -> R.string.family_roleTemplateOther
    }
)

@Composable
fun memberStatus(status: MemberStatus): String = stringResource(
    when (status) {
        MemberStatus.INVITED -> R.string.family_statusInvited
        MemberStatus.JOINED -> R.string.family_statusJoined
        MemberStatus.REMOVED, MemberStatus.OTHER -> R.string.family_statusOther
    }
)

/** 邀请码还剩多久；过了就说已过期。 */
@Composable
fun codeRemaining(code: MemberCode): String {
    if (code.expired) return stringResource(R.string.family_codeExpired)
    val remaining = Duration.between(
        Instant.now(),
        Instant.ofEpochMilli(code.expiresAtMs.toLong()),
    ).seconds.coerceAtLeast(0)
    val hours = remaining / 3600
    val minutes = (remaining % 3600) / 60
    val value = if (hours > 0) "${hours}h ${minutes}m" else "${minutes}m"
    return stringResource(R.string.family_codeExpires, value)
}

@Composable
fun permissionName(item: PermissionItem): String = stringResource(
    when (item) {
        PermissionItem.OPEN_APPS -> R.string.family_permissionOpenApps
        PermissionItem.PLUGIN_ADMIN -> R.string.family_permissionPluginAdmin
        PermissionItem.INSTALL_APPS -> R.string.family_permissionInstallApps
        PermissionItem.MODEL_SETTINGS -> R.string.family_permissionModelSettings
        PermissionItem.SERVER_SETTINGS -> R.string.family_permissionServerSettings
        PermissionItem.MANAGE_MEMBERS -> R.string.family_permissionManageMembers
        PermissionItem.ADD_FRIENDS -> R.string.family_permissionAddFriends
        PermissionItem.SHARE_APPS -> R.string.family_permissionShareApps
        PermissionItem.AI -> R.string.family_permissionAi
        PermissionItem.PRIVILEGED_TOOLS -> R.string.family_permissionPrivilegedTools
        PermissionItem.PRIVATE_BOT -> R.string.family_permissionPrivateBot
        PermissionItem.SHARED_BOTS -> R.string.family_permissionSharedBots
        PermissionItem.LEND_OWN_BOT -> R.string.family_permissionLendOwnBot
        PermissionItem.OWNER -> R.string.family_permissionOwner
        else -> R.string.family_permissionOther
    }
)

/** 加入页一项权限的写法：要管理员批的单独标出来。 */
@Composable
fun abilityText(ability: JoinAbility): String {
    val state = when (ability.state) {
        JoinAbilityState.ALLOWED -> null
        JoinAbilityState.NEEDS_ADMIN -> stringResource(R.string.family_gateAskAdmin)
        JoinAbilityState.NOT_ALLOWED -> stringResource(R.string.family_gateDeny)
        else -> null
    }
    val name = permissionName(ability.item)
    return if (state == null) name else "$name · $state"
}

val FamilyProblem.problem: Problem
    get() = when (reason) {
        FamilyFailure.TRANSPORT_UNAVAILABLE -> problem(
            R.string.problem_transport_unavailable_title,
            R.string.problem_transport_unavailable_next_step,
            message,
        )
        FamilyFailure.UNAUTHORIZED -> problem(
            R.string.problem_unauthorized_title,
            R.string.problem_unauthorized_next_step,
            message,
        )
        FamilyFailure.PERMISSION_REQUIRED -> problem(
            R.string.family_permissionRequiredTitle,
            R.string.family_permissionRequiredNextStep,
            message,
        )
        FamilyFailure.INVALID_INPUT -> problem(R.string.family_invalidInputTitle, detail = message)
        FamilyFailure.USERNAME_TAKEN -> problem(
            R.string.family_usernameTakenTitle,
            R.string.family_usernameTakenNextStep,
        )
        FamilyFailure.NOT_FOUND -> problem(
            R.string.family_notFoundTitle,
            R.string.family_notFoundNextStep,
            message,
        )
        FamilyFailure.REMOVED -> problem(R.string.family_removedTitle, R.string.family_removedNextStep)
        FamilyFailure.DEVICE_REGISTERED -> problem(
            R.string.family_deviceRegisteredTitle,
            R.string.family_deviceRegisteredNextStep,
        )
        FamilyFailure.NOT_READY -> problem(R.string.problem_not_ready_title, R.string.family_notReadyNextStep)
        FamilyFailure.MATRIX_UNAVAILABLE -> problem(
            R.string.family_matrixUnavailableTitle,
            R.string.family_matrixUnavailableNextStep,
            message,
        )
        FamilyFailure.STORAGE -> problem(
            R.string.problem_storage_title,
            R.string.problem_storage_next_step,
            message,
        )
        FamilyFailure.UPSTREAM -> problem(
            R.string.problem_upstream_title,
            R.string.problem_upstream_next_step,
            message,
        )
        FamilyFailure.PROTOCOL -> problem(R.string.family_protocolTitle, R.string.family_protocolNextStep)
        else -> problem(R.string.problem_upstream_title, R.string.problem_upstream_next_step, message)
    }

private fun problem(title: Int, nextStep: Int? = null, detail: String? = null): Problem =
    Problem(titleRes = title, nextStepRes = nextStep, detail = detail)
