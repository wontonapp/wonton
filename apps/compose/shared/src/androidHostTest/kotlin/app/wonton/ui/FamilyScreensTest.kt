package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.FamilyAccess
import app.wonton.protocol.FamilyViewModel
import app.wonton.protocol.JoinAbility
import app.wonton.protocol.JoinAbilityState
import app.wonton.protocol.JoinRequestRow
import app.wonton.protocol.JoinRequestsViewModel
import app.wonton.protocol.JoinRoleView
import app.wonton.protocol.MemberCode
import app.wonton.protocol.MemberRow
import app.wonton.protocol.MemberStatus
import app.wonton.protocol.MembersViewModel
import app.wonton.protocol.PermissionItem
import app.wonton.protocol.RoleTemplate
import app.wonton.protocol.RolesViewModel

/// 成员页和加入页的预览夹具（ADR-0088）：固定 ViewModel，浅色/深色各一份。
/// 截图测试（`FamilySnapshotsTest`）直接调用这些 composable。

@Preview(widthDp = 390, heightDp = 844, showBackground = true)
@Composable
fun membersLight() = PreviewFrame {
    Box(Modifier.fillMaxSize().padding(top = 40.dp)) {
        MembersContent(demoFamily(), {}, {}, {}, {}, {}, {}, {})
    }
}

@Preview(widthDp = 390, heightDp = 844, showBackground = true)
@Composable
fun membersDark() = PreviewFrame(dark = true) {
    Box(Modifier.fillMaxSize().padding(top = 40.dp)) {
        MembersContent(demoFamily(), {}, {}, {}, {}, {}, {}, {})
    }
}

@Preview(widthDp = 390, heightDp = 844, showBackground = true)
@Composable
fun joinPage() {
    PreviewFrame {
        JoinInviteScreen(role = demoJoinRole(), onAccept = {}, onDecline = {})
    }
}

fun demoJoinRole() = JoinRoleView(
    roleId = "family",
    name = null,
    template = RoleTemplate.FAMILY,
    abilities = listOf(
        JoinAbility(PermissionItem.OPEN_APPS, JoinAbilityState.ALLOWED),
        JoinAbility(PermissionItem.INSTALL_APPS, JoinAbilityState.NEEDS_ADMIN),
        JoinAbility(PermissionItem.ADD_FRIENDS, JoinAbilityState.ALLOWED),
        JoinAbility(PermissionItem.PRIVATE_BOT, JoinAbilityState.NOT_ALLOWED),
    ),
)

private fun demoFamily() = FamilyViewModel(
    access = FamilyAccess(
        userId = "@owner:demo",
        username = "owner",
        displayName = "Owner",
        isOwner = true,
        roleId = null,
        roleName = null,
        permissions = null,
    ),
    members = MembersViewModel(
        loading = false,
        members = listOf(
            demoMember("lin", "Lin", "family", MemberStatus.JOINED),
            demoMember("grace", "Grace", "friend", MemberStatus.INVITED),
        ),
        problem = null,
    ),
    joinRequests = JoinRequestsViewModel(
        loading = false,
        requests = listOf(
            JoinRequestRow(
                id = "req-mika",
                username = "mika",
                displayName = "Mika",
                roleId = "friend",
                roleName = null,
                roleTemplate = RoleTemplate.FRIEND,
                deviceName = "Pixel",
                requestedAtMs = 1_700_000_000_000u,
            ),
        ),
        problem = null,
    ),
    roles = RolesViewModel(
        loading = false,
        roles = emptyList(),
        defaultInviteRole = "friend",
        problem = null,
    ),
    invite = null,
)

private fun demoMember(username: String, display: String, role: String, status: MemberStatus) = MemberRow(
    username = username,
    userId = "@$username:demo",
    displayName = display,
    email = null,
    status = status,
    roleId = role,
    roleName = null,
    roleTemplate = if (role == "family") RoleTemplate.FAMILY else RoleTemplate.FRIEND,
    privateBot = null,
    code = if (status == MemberStatus.INVITED) {
        MemberCode(
            expiresAtMs = System.currentTimeMillis().toULong() + 48u * 3600_000u,
            expired = false,
        )
    } else {
        null
    },
)
