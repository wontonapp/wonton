package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.client.pluginAudienceNotice
import app.wonton.client.selectedPluginAudience
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginAudienceNotice
import app.wonton.protocol.PluginAudienceView
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.InstalledPlugin
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

@Composable
internal fun PluginAudienceSection(
    audience: PluginAudienceView,
    enabled: Boolean = true,
    onChange: (PluginAudienceView) -> Unit,
) {
    DetailSection(stringResource(R.string.install_audience_title)) {
        audience.choices.forEach { choice ->
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(audienceRoleName(choice), Modifier.weight(1f), style = WontonTheme.typography.body)
                WontonSwitch(choice.required || choice.selected, { selected ->
                    onChange(audience.copy(choices = audience.choices.map {
                        if (it.id == choice.id) it.copy(selected = selected) else it
                    }))
                }, enabled && !choice.required)
            }
        }
        if (pluginAudienceNotice(audience) == PluginAudienceNotice.SHARED_DATA) {
            WontonNotice(WontonIcon.Warning, stringResource(R.string.install_audience_shared_data), isError = false)
        }
    }
}

@Composable
private fun audienceRoleName(choice: app.wonton.protocol.PluginAudienceChoice): String = when (choice.template) {
    app.wonton.protocol.RoleTemplate.ADMIN -> stringResource(R.string.install_audience_admin)
    app.wonton.protocol.RoleTemplate.FAMILY -> stringResource(R.string.install_audience_family)
    app.wonton.protocol.RoleTemplate.CHILD -> stringResource(R.string.install_audience_child)
    app.wonton.protocol.RoleTemplate.FRIEND -> stringResource(R.string.install_audience_friend)
    app.wonton.protocol.RoleTemplate.OTHER, null -> choice.name
}

@Composable
internal fun InstalledPluginAudience(
    plugins: PluginsState,
    id: String,
    view: PluginAudienceView,
    onUpdated: (PluginDetail) -> Unit,
) {
    var audience by remember(view) { mutableStateOf(view) }
    var busy by remember(id) { mutableStateOf(false) }
    var problem by remember(id) { mutableStateOf<Problem?>(null) }
    val scope = rememberCoroutineScope()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        PluginAudienceSection(audience, !busy) { audience = it }
        WontonButton(stringResource(R.string.common_save), {
            val selected = selectedPluginAudience(audience) ?: return@WontonButton
            busy = true
            scope.launch {
                try {
                    onUpdated(plugins.core.updatePluginAudience(Action.SetPluginAudience(id, selected)))
                    problem = null
                } catch (failure: CoreException) {
                    problem = failure.problem
                }
                busy = false
            }
        }, enabled = !busy && audience != view)
        problem?.let { ProblemNotice(it) }
    }
}

/** 已装应用的“谁能用”一行：主人能编辑，别人只读；没有开放范围就不显示。 */
@Composable
internal fun PluginAudienceView?.audienceRow(
    plugins: PluginsState,
    id: String,
    onUpdated: (PluginDetail) -> Unit,
) {
    val audience = this ?: return
    if (audience.choices.isEmpty()) {
        PluginAudienceSection(audience, enabled = false, onChange = {})
    } else {
        InstalledPluginAudience(plugins, id, audience, onUpdated)
    }
}
