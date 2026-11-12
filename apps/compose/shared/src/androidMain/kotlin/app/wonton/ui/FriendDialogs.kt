package app.wonton.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.wonton.protocol.FriendBlockScope
import app.wonton.protocol.FriendFailure
import app.wonton.protocol.FriendGroup
import app.wonton.protocol.FriendProblem
import app.wonton.protocol.FriendRow
import app.wonton.protocol.FriendsState
import app.wonton.protocol.FriendsViewModel
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Expand
import app.wonton.ui.icons.QrCode
import app.wonton.ui.icons.WontonIcon

private const val MILLIS_PER_SECOND = 1_000L

private val QR_SIZE = 260.dp

@Composable
internal fun AddFriendSheet(
    myLink: String?,
    onScan: () -> Unit,
    onShowMyCode: () -> Unit,
    onSubmit: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var userId by rememberSaveable { mutableStateOf("") }
    WontonSheet(onDismiss, stringResource(R.string.friends_add_title)) { close ->
        val submit: () -> Unit = {
            val id = userId.trim()
            if (id.isNotEmpty()) close { onSubmit(id) }
        }
        Column(
            Modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            WontonTextField(
                userId,
                { userId = it },
                Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.friends_add_placeholder),
                label = stringResource(R.string.friends_add_title),
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Email,
                    imeAction = ImeAction.Send,
                ),
                keyboardActions = KeyboardActions(onSend = { submit() }),
            )
            WontonButton(
                stringResource(R.string.friends_add_submit),
                submit,
                Modifier.fillMaxWidth(),
                enabled = userId.isNotBlank(),
            )
        }
        WontonGroup {
            SheetRow(WontonIcon.Expand, stringResource(R.string.friends_scan_code)) { close(onScan) }
            myLink?.let {
                WontonSeparator()
                SheetRow(WontonIcon.QrCode, stringResource(R.string.friends_my_code)) { close(onShowMyCode) }
            }
        }
    }
}

/** 出示我的码：把核心给的 matrix.to 用户链接画成二维码，对面扫了或粘贴就是发好友请求。 */
@Composable
internal fun MyFriendCodeDialog(link: String, onDismiss: () -> Unit) {
    WontonDialog(
        stringResource(R.string.friends_my_code),
        onDismiss = onDismiss,
        detail = {
            Column(
                Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                val qr = remember(link) { qrBitmap(link) }
                Image(
                    bitmap = qr,
                    contentDescription = stringResource(R.string.friends_my_code),
                    modifier = Modifier.size(QR_SIZE).clip(RoundedCornerShape(WontonTheme.radii.control)),
                    filterQuality = FilterQuality.None,
                )
                SelectionContainer {
                    Text(
                        link,
                        Modifier.fillMaxWidth()
                            .background(
                                WontonTheme.colors.surfaceSubtle.toColor(),
                                RoundedCornerShape(WontonTheme.radii.control),
                            )
                            .padding(WontonTheme.spacing.s12),
                        style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                        color = WontonTheme.colors.textPrimary.toColor(),
                    )
                }
            }
        },
    )
}

/** 操作项：和 WontonActionSheet 的行一个样子，供自带表单的面板接着用。 */
@Composable
private fun SheetRow(icon: ImageVector, label: String, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.pageInset),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            icon,
            // 装饰：操作项有文字。
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textPrimary.toColor(),
        )
        Text(label, style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
    }
}

@Composable
internal fun GroupNameDialog(
    title: String,
    initial: String,
    onDismiss: () -> Unit,
    onConfirm: (String) -> Unit,
) {
    var name by rememberSaveable(title) { mutableStateOf(initial) }
    WontonDialog(
        title,
        onDismiss = onDismiss,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(stringResource(R.string.common_save), WontonButtonKind.PRIMARY) {
                name.trim().takeIf(String::isNotEmpty)?.let(onConfirm)
            },
        ),
        detail = {
            WontonTextField(
                name,
                { name = it },
                Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.friends_group_name_placeholder),
                label = title,
            )
        },
    )
}

@Composable
internal fun ChooseGroupsSheet(
    row: FriendRow,
    groups: List<FriendGroup>,
    onSave: (Set<String>) -> Unit,
    onDismiss: () -> Unit,
) {
    var selected by rememberSaveable(row.userId) { mutableStateOf(row.groups.toSet()) }
    WontonSheet(onDismiss, stringResource(R.string.friends_choose_groups_title, row.name())) { close ->
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            WontonGroup {
                groups.forEachIndexed { index, group ->
                    if (index > 0) WontonSeparator()
                    GroupChoiceRow(group.name, group.id in selected) { checked ->
                        selected = if (checked) selected + group.id else selected - group.id
                    }
                }
            }
            WontonButton(
                stringResource(R.string.common_save),
                { close { onSave(selected) } },
                Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.pageInset),
            )
        }
    }
}

@Composable
private fun GroupChoiceRow(name: String, checked: Boolean, onCheckedChange: (Boolean) -> Unit) {
    androidx.compose.foundation.layout.Row(
        Modifier.fillMaxWidth()
            .toggleable(checked, role = Role.Checkbox, onValueChange = onCheckedChange)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
    ) {
        Text(
            name,
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        if (checked) {
            Icon(
                WontonIcon.Check,
                contentDescription = null,
                tint = WontonTheme.colors.primary.toColor(),
            )
        }
    }
}

@Composable
internal fun BlockServerDialog(onDismiss: () -> Unit, onConfirm: (String) -> Unit) {
    var server by rememberSaveable { mutableStateOf("") }
    WontonDialog(
        stringResource(R.string.friends_block_server_title),
        onDismiss = onDismiss,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(stringResource(R.string.friends_block_submit), WontonButtonKind.PRIMARY) {
                server.trim().takeIf(String::isNotEmpty)?.let(onConfirm)
            },
        ),
        detail = {
            WontonTextField(
                server,
                { server = it },
                Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.friends_block_server_placeholder),
                label = stringResource(R.string.friends_block_server_title),
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Uri,
                    imeAction = ImeAction.Done,
                ),
                keyboardActions = KeyboardActions(onDone = {
                    server.trim().takeIf(String::isNotEmpty)?.let(onConfirm)
                }),
            )
        },
    )
}

@Composable
internal fun FriendsState.Failed.problem(): Problem = when (problem.reason) {
    FriendFailure.TRANSPORT_UNAVAILABLE -> Problem(
        titleRes = R.string.friends_error_transport,
        nextStepRes = R.string.problem_transport_unavailable_next_step,
        detail = problem.detail(),
    )
    FriendFailure.UNAUTHORIZED -> Problem(
        titleRes = R.string.friends_error_unauthorized,
        nextStepRes = R.string.friends_error_unauthorized_next,
        detail = problem.detail(),
    )
    FriendFailure.FORBIDDEN -> Problem(
        titleRes = R.string.friends_error_forbidden,
        nextStepRes = R.string.friends_error_forbidden_next,
        detail = problem.detail(),
    )
    FriendFailure.INVALID_USER_ID -> Problem(
        titleRes = R.string.friends_error_invalid_user_id,
        nextStepRes = R.string.friends_error_invalid_user_id_next,
        detail = problem.detail(),
    )
    FriendFailure.REQUEST_NOT_FOUND -> Problem(
        titleRes = R.string.friends_error_request_not_found,
        detail = problem.detail(),
    )
    FriendFailure.RATE_LIMITED -> {
        val seconds = problem.retryAfterMs?.toLong()?.div(MILLIS_PER_SECOND)?.coerceAtLeast(1L)
        Problem(
            title = seconds?.let { stringResource(R.string.friends_error_rate_limited, it) }
                ?: stringResource(R.string.friends_error_rate_limited_later),
            detail = problem.detail(),
        )
    }
    FriendFailure.UPSTREAM -> Problem(titleRes = R.string.friends_error_upstream, detail = problem.detail())
    FriendFailure.OTHER -> Problem(titleRes = R.string.friends_error_other, detail = problem.detail())
}

private fun FriendProblem.detail(): String? = listOfNotNull(
    httpStatus?.let { "HTTP $it" },
    matrixErrcode,
).takeIf { it.isNotEmpty() }?.joinToString(" · ")

internal fun FriendsViewModel.blocks() =
    blocked.filter { it.scope != FriendBlockScope.OTHER }.sortedBy { it.scope.ordinal }

internal fun FriendRow.name(): String = displayName?.takeIf(String::isNotBlank) ?: userId
