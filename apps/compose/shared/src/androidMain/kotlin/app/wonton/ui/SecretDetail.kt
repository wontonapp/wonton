package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.material3.Text
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.SecretInput
import app.wonton.protocol.SecretInputState
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.launch

/** 要填的密钥：只有主人看得到这一节；点开可改，可选项可以清空。改完核心会重建用到它的服务。 */
@Composable
internal fun SecretSection(detail: PluginDetail, plugins: PluginsState, onUpdated: (PluginDetail) -> Unit) {
    if (detail.secretInputs.isEmpty()) return
    val owner by produceState<Boolean?>(null) { value = runCatching { plugins.core.me().isOwner }.getOrNull() }
    if (owner != true) return
    DetailSection(stringResource(R.string.secret_section_title)) {
        detail.secretInputs.forEachIndexed { index, item ->
            SecretRow(detail.id, item, plugins, onUpdated)
            if (index < detail.secretInputs.lastIndex) WontonSeparator()
        }
    }
}

@Composable
private fun SecretRow(
    pluginId: String,
    item: SecretInputState,
    plugins: PluginsState,
    onUpdated: (PluginDetail) -> Unit,
) {
    val colors = WontonTheme.colors
    val scope = rememberCoroutineScope()
    var editing by remember(item.name) { mutableStateOf(false) }
    var draft by remember(item.name) { mutableStateOf("") }
    var busy by remember(item.name) { mutableStateOf(false) }
    var problem by remember(item.name) { mutableStateOf<Problem?>(null) }
    val changed = item.changedAtMs?.let {
        DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(it.toLong()))
    }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(item.title, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
                Text(
                    secretStateText(item, changed),
                    style = WontonTheme.typography.caption,
                    color = colors.textSecondary.toColor(),
                )
            }
            if (!item.required) {
                Text(
                    stringResource(R.string.secret_optional),
                    style = WontonTheme.typography.caption,
                    color = colors.textSecondary.toColor(),
                )
            }
            if (!editing) {
                WontonSmallButton(stringResource(R.string.secret_edit), { editing = true }, enabled = !busy)
            }
        }
        if (editing) {
            SecretInputField(
                SecretInput(item.name, item.title, null, null, item.required),
                draft,
                !busy,
            ) { draft = it }
            Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                WontonSmallButton(
                    stringResource(R.string.common_save),
                    {
                        busy = true
                        problem = null
                        scope.launch {
                            try {
                                onUpdated(plugins.core.setPluginSecret(pluginId, item.name, draft))
                                editing = false
                                draft = ""
                            } catch (failure: CoreException) {
                                problem = failure.problem
                            }
                            busy = false
                        }
                    },
                    enabled = !busy && draft.isNotBlank(),
                )
                if (!item.required) {
                    WontonSmallButton(
                        stringResource(R.string.secret_clear),
                        {
                            busy = true
                            problem = null
                            scope.launch {
                                try {
                                    onUpdated(plugins.core.setPluginSecret(pluginId, item.name, null))
                                    editing = false
                                    draft = ""
                                } catch (failure: CoreException) {
                                    problem = failure.problem
                                }
                                busy = false
                            }
                        },
                        enabled = !busy,
                    )
                }
                WontonSmallButton(stringResource(R.string.common_cancel), {
                    editing = false
                    draft = ""
                    problem = null
                })
            }
            problem?.let { ProblemNotice(it) }
        }
    }
}

