package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.CoreException
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 详情页的一行开关：网页用摄像头和麦克风的记下的决定，没问过不显示。
 *  关掉是改回没问过（下次再问）；拒绝只在询问弹窗里产生。 */
@Composable
internal fun MediaPermissionSection(plugins: PluginsState, id: String) {
    val grant = plugins.viewModel.mediaGrants.firstOrNull { it.pluginId == id } ?: return
    val title = stringResource(R.string.plugin_media_title)
    var busy by remember(id) { mutableStateOf(false) }
    var problem by remember(id) { mutableStateOf<Problem?>(null) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        WontonGroup {
            Row(
                Modifier.fillMaxWidth()
                    .heightIn(min = WontonTheme.size.target)
                    .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            ) {
                Text(
                    title,
                    Modifier.weight(1f),
                    style = WontonTheme.typography.body,
                    color = WontonTheme.colors.textPrimary.toColor(),
                )
                WontonSwitch(
                    checked = grant.allowed,
                    onCheckedChange = { allowed ->
                        busy = true
                        problem = null
                        // 本机设置，同步落盘；和别的详情页开关一样失败弹回并显示原因。
                        problem = try {
                            plugins.core.setPluginMediaPermission(pluginId = id, allow = allowed)
                            null
                        } catch (failure: CoreException) {
                            failure.problem
                        }
                        busy = false
                    },
                    enabled = !busy,
                    modifier = Modifier.semantics { contentDescription = title },
                )
            }
        }
        problem?.let { ProblemNotice(it) }
    }
}
