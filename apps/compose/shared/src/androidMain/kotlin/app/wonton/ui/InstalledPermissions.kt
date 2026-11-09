package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.CoreException
import app.wonton.protocol.PermissionLine
import app.wonton.protocol.PermissionOpened
import app.wonton.protocol.PluginDetail
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import java.text.DateFormat
import java.util.Date
import java.util.Locale
import androidx.compose.ui.platform.LocalConfiguration
import kotlinx.coroutines.launch

@Composable
internal fun PluginPermissionSection(
    detail: PluginDetail,
    plugins: PluginsState,
    onUpdated: (PluginDetail) -> Unit,
) {
    if (detail.permissions.isEmpty()) return
    var changing by remember(detail.id) { mutableStateOf<String?>(null) }
    DetailSection(stringResource(R.string.install_permissions)) {
        detail.permissions.forEach { line ->
            key(detail.id, line.permission) {
                if (line.optional && detail.installed != null) {
                    InstalledPermission(
                        plugins, detail.id, line, changing == null, onUpdated,
                        onBusy = { busy -> changing = if (busy) line.permission else null },
                    )
                } else {
                    PermissionRow(line)
                }
            }
        }
    }
}

@Composable
private fun InstalledPermission(
    plugins: PluginsState,
    id: String,
    line: PermissionLine,
    enabled: Boolean,
    onUpdated: (PluginDetail) -> Unit,
    onBusy: (Boolean) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var pending by remember { mutableStateOf<Boolean?>(null) }
    var busy by remember { mutableStateOf(false) }
    var failedTarget by remember { mutableStateOf<Boolean?>(null) }
    var problem by remember { mutableStateOf<Problem?>(null) }
    val change: (Boolean) -> Unit = { target ->
        busy = true
        onBusy(true)
        problem = null
        scope.launch {
            try {
                onUpdated(plugins.core.setPluginPermission(id, line.permission, target))
                failedTarget = null
            } catch (failure: CoreException) {
                failedTarget = target
                problem = failure.problem
            } finally {
                busy = false
                onBusy(false)
            }
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        PermissionRow(line) {
            if (busy) {
                CircularProgressIndicator(Modifier.size(WontonTheme.size.iconSmall), strokeWidth = 2.dp)
            } else {
                WontonSwitch(
                    line.enabled, { pending = it }, enabled && line.supported,
                    Modifier.semantics { contentDescription = line.text },
                )
            }
        }
        line.opened?.let { PermissionSupportingText(permissionOpenedText(it)) }
        problem?.let { failure ->
            ProblemNotice(failure)
            WontonButton(
                stringResource(R.string.common_retry),
                { failedTarget?.let(change) },
                kind = WontonButtonKind.SECONDARY,
                enabled = enabled && !busy,
            )
        }
    }
    pending?.let { target ->
        PermissionChangeConfirmation(line, target, { pending = null }) {
            pending = null
            change(target)
        }
    }
}

@Composable
private fun permissionOpenedText(opened: PermissionOpened): String {
    val locale = LocalConfiguration.current.locales[0] ?: Locale.getDefault()
    val date = DateFormat.getDateInstance(DateFormat.SHORT, locale).format(Date(opened.since.toLong() * 1000))
    return opened.bot?.let { bot ->
        stringResource(R.string.plugin_permission_opened_bot, bot.title, date)
    } ?: stringResource(R.string.plugin_permission_opened_owner, date)
}
