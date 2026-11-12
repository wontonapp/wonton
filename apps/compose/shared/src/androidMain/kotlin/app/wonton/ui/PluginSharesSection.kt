package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.CoreException
import app.wonton.protocol.FriendsState
import app.wonton.protocol.FriendsViewModel
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.PluginShares
import app.wonton.protocol.ShareGrant
import app.wonton.protocol.ShareRecipient
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.launch

@Composable
internal fun PluginSharesSection(
    pluginId: String, shares: PluginShares, plugins: PluginsState, onUpdated: (PluginDetail) -> Unit,
) {
    var recipients by remember(pluginId) { mutableStateOf<FriendsViewModel?>(null) }
    var problem by remember(pluginId) { mutableStateOf<Problem?>(null) }
    var attempt by remember(pluginId) { mutableIntStateOf(0) }
    var editing by remember(pluginId) { mutableStateOf(false) }
    var grant by remember(pluginId) { mutableStateOf<ShareGrant?>(null) }
    var revoking by remember(pluginId) { mutableStateOf<String?>(null) }
    var failedRevoke by remember(pluginId) { mutableStateOf<ShareGrant?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(pluginId, attempt) {
        recipients = null
        problem = null
        try {
            val next = plugins.core.shareRecipients()
            recipients = next
            problem = (next.state as? FriendsState.Failed)?.problem?.let { friendShareProblem(it) }
        } catch (failure: CoreException) {
            problem = failure.problem
        }
    }
    val revoke: (ShareGrant) -> Unit = { target ->
        revoking = target.id
        problem = null
        failedRevoke = null
        scope.launch {
            try {
                onUpdated(plugins.core.revokePluginShare(target.id, pluginId))
            } catch (failure: CoreException) {
                problem = failure.problem
                failedRevoke = target
            }
            revoking = null
        }
    }
    DetailSection(stringResource(R.string.share_title)) {
        val count = shares.friendCount.toInt()
        Text(pluralStringResource(R.plurals.share_count, count, count),
            style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
        shares.grants.forEach { existing ->
            ShareGrantRow(existing, recipients, revoking == null,
                onEdit = { grant = existing; editing = true }, onRevoke = { revoke(existing) })
        }
        WontonButton(stringResource(R.string.share_add), { grant = null; editing = true },
            kind = WontonButtonKind.SECONDARY, enabled = revoking == null)
        problem?.let {
            ProblemNotice(it)
            val target = failedRevoke
            WontonButton(stringResource(R.string.common_retry), {
                if (target != null) revoke(target) else attempt++
            }, kind = WontonButtonKind.SECONDARY, enabled = revoking == null)
        }
    }
    if (editing) ShareGrantEditor(pluginId, shares.perUserAccounts, grant, plugins,
        onUpdated = onUpdated, onDismiss = { editing = false })
}

@Composable
private fun ShareGrantRow(
    grant: ShareGrant, recipients: FriendsViewModel?, enabled: Boolean, onEdit: () -> Unit, onRevoke: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(shareRecipientName(grant.recipient, recipients), style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor())
        Text(shareExpiryText(grant.expiresAtMs), style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor())
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
            WontonButton(stringResource(R.string.share_edit), onEdit,
                kind = WontonButtonKind.SECONDARY, enabled = enabled)
            WontonButton(stringResource(R.string.share_revoke), onRevoke,
                kind = WontonButtonKind.DANGER, enabled = enabled)
        }
    }
}

@Composable
internal fun shareRecipientName(recipient: ShareRecipient, recipients: FriendsViewModel?): String = when (recipient) {
    is ShareRecipient.Friend -> recipients?.friends?.firstOrNull { it.userId == recipient.userId }
        ?.displayName ?: recipient.userId
    is ShareRecipient.Group -> recipients?.groups?.firstOrNull { it.id == recipient.groupId }?.name
        ?: recipient.groupId
    ShareRecipient.Other -> stringResource(R.string.share_unknown)
}

@Composable
internal fun shareExpiryText(expiresAtMs: ULong?): String = expiresAtMs?.let {
    DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(it.toLong()))
} ?: stringResource(R.string.share_indefinite)

internal fun FriendsViewModel.recipientChoices(): List<ShareRecipient> =
    if (state is FriendsState.Ready) {
        friends.map { ShareRecipient.Friend(it.userId) } + groups.map { ShareRecipient.Group(it.id) }
    } else emptyList()
