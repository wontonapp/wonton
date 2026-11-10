package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontFamily
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.OwnerSecretFailure
import app.wonton.protocol.OwnerSecretProblem
import app.wonton.protocol.OwnerSecretView
import app.wonton.protocol.OwnerSecretsView
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.Role
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.CancellationException

private const val EXPANDED_ARROW_DEGREES = 90f

@Composable
internal fun OwnerSecretsSection(pluginId: String, plugins: PluginsState) {
    var identityAttempt by remember(plugins.core, pluginId) { mutableIntStateOf(0) }
    val identity by produceState<Result<Role>?>(null, plugins.core, pluginId, identityAttempt) {
        value = null
        value = try {
            Result.success(plugins.core.me().role)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    val result = identity
    when {
        result == null -> OwnerSecretLoading()
        result.isFailure -> ProblemRow(result.failureProblem) { identityAttempt++ }
        result.getOrThrow() == Role.OWNER -> OwnerSecretsGroup(pluginId, plugins)
    }
}

@Composable
private fun OwnerSecretsGroup(pluginId: String, plugins: PluginsState) {
    DisposableEffect(plugins, pluginId) {
        onDispose { plugins.send(Action.DismissOwnerSecrets(pluginId)) }
    }
    val view = plugins.viewModel.ownerSecrets?.takeIf { it.pluginId == pluginId }
    if (view?.secrets?.any { it.expanded && it.value != null } == true) SecureScreen()
    WontonGroup {
        OwnerSecretDisclosure(
            stringResource(R.string.owner_secrets_title),
            view != null,
            Modifier.padding(horizontal = WontonTheme.spacing.s16),
        ) {
            plugins.send(
                if (view == null) Action.ListOwnerSecrets(pluginId) else Action.DismissOwnerSecrets(pluginId),
            )
        }
        if (view != null) {
            WontonSeparator()
            Column(
                Modifier.padding(WontonTheme.spacing.s16),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            ) {
                OwnerSecretsContent(view, plugins)
            }
        }
    }
}

@Composable
private fun OwnerSecretsContent(view: OwnerSecretsView, plugins: PluginsState) {
    val problem = view.problem
    when {
        view.loading -> OwnerSecretLoading()
        problem != null -> ProblemRow(problem.ownerSecretProblem()) {
            plugins.send(Action.ListOwnerSecrets(view.pluginId))
        }
        view.secrets.isEmpty() -> Text(
            stringResource(R.string.owner_secrets_empty),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        else -> view.secrets.forEachIndexed { index, item ->
            key(item.name) { OwnerSecretRow(view.pluginId, item, plugins) }
            if (index < view.secrets.lastIndex) WontonSeparator()
        }
    }
}

@Composable
private fun OwnerSecretRow(pluginId: String, item: OwnerSecretView, plugins: PluginsState) {
    val context = LocalContext.current
    val clipLabel = stringResource(R.string.owner_secrets_title)
    val problem = item.problem
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        OwnerSecretDisclosure(
            item.label,
            item.expanded,
            actionLabel = stringResource(
                if (item.expanded) R.string.owner_secrets_hide else R.string.owner_secrets_show,
            ),
        ) {
            plugins.send(
                if (item.expanded) Action.HideOwnerSecret(pluginId, item.name)
                else Action.ShowOwnerSecret(pluginId, item.name),
            )
        }
        if (item.expanded) {
            when {
                item.loading -> OwnerSecretLoading()
                problem != null -> ProblemRow(problem.ownerSecretProblem()) {
                    plugins.send(Action.ShowOwnerSecret(pluginId, item.name))
                }
                else -> item.value?.let { value ->
                    // Selection would bypass the sensitive clipboard flag; copying goes through the button only.
                    Text(
                        value,
                        Modifier.fillMaxWidth(),
                        style = WontonTheme.typography.body.copy(fontFamily = FontFamily.Monospace),
                        color = WontonTheme.colors.textPrimary.toColor(),
                    )
                    WontonSmallButton(stringResource(R.string.message_copy), {
                        copySensitive(context, value, clipLabel)
                    })
                }
            }
        }
    }
}

@Composable
private fun OwnerSecretDisclosure(
    label: String,
    expanded: Boolean,
    modifier: Modifier = Modifier,
    actionLabel: String? = null,
    onClick: () -> Unit,
) {
    val state = stringResource(if (expanded) R.string.appearance_expanded else R.string.appearance_collapsed)
    Row(
        modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .semantics { stateDescription = state }
            .clickable(role = androidx.compose.ui.semantics.Role.Button, onClick = onClick)
            .padding(vertical = WontonTheme.spacing.s8),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            label,
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        if (actionLabel != null) {
            Text(
                actionLabel,
                style = WontonTheme.typography.supporting,
                color = WontonTheme.colors.textSecondary.toColor(),
            )
        } else {
            Icon(
                WontonIcon.Forward,
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.iconTiny)
                    .rotate(if (expanded) EXPANDED_ARROW_DEGREES else 0f),
                tint = WontonTheme.colors.textSecondary.toColor(),
            )
        }
    }
}

@Composable
private fun OwnerSecretLoading() {
    Row(
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(
            Modifier.size(WontonTheme.size.iconSmall),
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        Text(
            stringResource(R.string.problem_loading),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

private fun OwnerSecretProblem.ownerSecretProblem(): Problem = when (reason) {
    OwnerSecretFailure.TRANSPORT_UNAVAILABLE -> PluginFailure.TRANSPORT_UNAVAILABLE.problem(message)
    OwnerSecretFailure.UNAUTHENTICATED -> PluginFailure.UNAUTHORIZED.problem(message)
    OwnerSecretFailure.UPSTREAM -> PluginFailure.UPSTREAM.problem(message)
    else -> Problem(
        titleRes = when (reason) {
            OwnerSecretFailure.CROSS_SITE -> R.string.owner_secrets_problem_cross_site
            OwnerSecretFailure.OWNER_ONLY -> R.string.owner_secrets_problem_owner_only
            OwnerSecretFailure.NOT_INSTALLED -> R.string.owner_secrets_problem_not_installed
            OwnerSecretFailure.NOT_FOUND -> R.string.owner_secrets_problem_not_found
            OwnerSecretFailure.NOT_READY -> R.string.owner_secrets_problem_not_ready
            OwnerSecretFailure.PROTOCOL -> R.string.owner_secrets_problem_protocol
            else -> R.string.owner_secrets_problem_other
        },
        detail = message.takeIf { it.isNotBlank() },
    )
}
