package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.CoreException
import app.wonton.protocol.FriendsState
import app.wonton.protocol.FriendsViewModel
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.ShareGrant
import app.wonton.protocol.ShareGrantUpdate
import app.wonton.protocol.ShareRecipient
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

@Composable
internal fun ShareGrantEditor(
    pluginId: String, friendsSupported: Boolean, grant: ShareGrant?, plugins: PluginsState,
    onUpdated: (PluginDetail) -> Unit, onDismiss: () -> Unit,
) {
    var recipients by remember { mutableStateOf<FriendsViewModel?>(null) }
    var problem by remember { mutableStateOf<Problem?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    var recipient by remember { mutableStateOf(grant?.recipient) }
    var expires by remember { mutableStateOf(grant?.expiresAtMs) }
    var acknowledged by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(attempt) {
        recipients = null
        problem = null
        try {
            val next = plugins.core.shareRecipients()
            recipients = next
            problem = (next.state as? FriendsState.Failed)?.problem?.let { friendShareProblem(it) }
                ?: if (next.state is FriendsState.Other) Problem(titleRes = R.string.share_problem_other) else null
        } catch (failure: CoreException) {
            problem = failure.problem
        }
    }
    val choices = recipients?.recipientChoices().orEmpty()
    val canSave = !busy && recipient in choices && (friendsSupported || acknowledged)
    val save = {
        val selected = recipient
        if (selected != null && canSave) {
            busy = true
            problem = null
            scope.launch {
                try {
                    onUpdated(plugins.core.savePluginShare(grant?.id,
                        ShareGrantUpdate(pluginId, selected, expires, acknowledged)))
                    onDismiss()
                } catch (failure: CoreException) {
                    problem = failure.problem
                }
                busy = false
            }
        }
    }
    WontonSheet(onDismiss = { if (!busy) onDismiss() },
        title = stringResource(if (grant == null) R.string.share_add else R.string.share_edit)) {
        ShareGrantFields(recipients, problem, friendsSupported, busy, recipient, expires, acknowledged, canSave,
            { recipient = it }, { expires = it }, { acknowledged = it }, { attempt++ }, save, onDismiss)
    }
}

@Composable
private fun ShareGrantFields(
    recipients: FriendsViewModel?, problem: Problem?, friendsSupported: Boolean, busy: Boolean,
    recipient: ShareRecipient?, expires: ULong?, acknowledged: Boolean, canSave: Boolean,
    onRecipient: (ShareRecipient) -> Unit, onExpires: (ULong?) -> Unit, onAcknowledged: (Boolean) -> Unit,
    onRetry: () -> Unit, onSave: () -> Unit, onDismiss: () -> Unit,
) {
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        when {
            recipients == null && problem == null -> LoadingView()
            recipients == null -> problem?.let { ProblemView(it, onRetry) }
            else -> {
                ShareRecipientChoices(recipients, recipient, !busy, onRecipient)
                ShareExpiryPicker(expires, !busy, onExpires)
                if (!friendsSupported) ShareRiskAcknowledgement(acknowledged, !busy, onAcknowledged)
                problem?.let { ProblemNotice(it) }
                val ready = recipients.state is FriendsState.Ready
                if (!ready) {
                    WontonButton(stringResource(R.string.common_retry), onRetry,
                        kind = WontonButtonKind.SECONDARY, enabled = !busy)
                }
                if (problem != null && ready) {
                    WontonButton(stringResource(R.string.common_retry), onSave,
                        kind = WontonButtonKind.SECONDARY, enabled = canSave)
                }
                WontonButton(stringResource(R.string.share_save), onSave, enabled = canSave)
            }
        }
        WontonButton(stringResource(R.string.share_cancel), onDismiss,
            kind = WontonButtonKind.SECONDARY, enabled = !busy)
    }
}

@Composable
private fun ShareRecipientChoices(
    recipients: FriendsViewModel, selected: ShareRecipient?, enabled: Boolean, onSelect: (ShareRecipient) -> Unit,
) {
    Text(stringResource(R.string.share_recipient), style = WontonTheme.typography.sectionTitle,
        color = WontonTheme.colors.textPrimary.toColor())
    if (recipients.state is FriendsState.Loading) {
        LoadingView()
        return
    }
    if (recipients.recipientChoices().isEmpty()) {
        Text(stringResource(R.string.share_empty), style = WontonTheme.typography.body,
            color = WontonTheme.colors.textSecondary.toColor())
    }
    if (recipients.state !is FriendsState.Ready) return
    if (recipients.friends.isNotEmpty()) {
        Text(stringResource(R.string.share_friends), style = WontonTheme.typography.sectionTitle)
        recipients.friends.forEach { friend ->
            val choice = ShareRecipient.Friend(friend.userId)
            WontonRadioRow(friend.displayName ?: friend.userId, selected == choice, enabled,
                { onSelect(choice) }, value = friend.userId.takeIf { friend.displayName != null })
        }
    }
}

@Composable
private fun ShareRiskAcknowledgement(acknowledged: Boolean, enabled: Boolean, onChange: (Boolean) -> Unit) {
    WontonNotice(WontonIcon.Warning, stringResource(R.string.share_risk))
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        Text(stringResource(R.string.share_acknowledge), Modifier.weight(1f),
            style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
        WontonSwitch(acknowledged, onChange, enabled)
    }
}
