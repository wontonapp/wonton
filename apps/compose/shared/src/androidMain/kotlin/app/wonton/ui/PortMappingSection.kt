package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
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
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.wonton.protocol.Action
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.PortMappingState
import app.wonton.protocol.PortProtocol
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/**
 * “映射到本机”一组（ADR-0072）：一张分组卡片里放开关和同号的端口，没映射成功的原因提示在卡片下面。
 * 核心没给映射状态时（太旧或没实现），端口按清单声明的列出来。
 *
 * 打开开关时顺带申请通知权限；拒绝了映射照样开，卡片下面如实提示通知栏看不到。
 */
@Composable
internal fun PortMappingSection(plugins: PluginsState, installed: InstalledPlugin) {
    val mapping = plugins.viewModel.portMappings.firstOrNull { it.pluginId == installed.id }
    var problem by remember(installed.id) { mutableStateOf<Problem?>(null) }
    val permission = LocalNotificationPermission.current
    var notificationGranted by remember(installed.id) {
        mutableStateOf(permission?.granted ?: true)
    }
    LifecycleResumeEffect(permission) {
        notificationGranted = permission?.granted ?: true
        onPauseOrDispose {}
    }
    val title = stringResource(R.string.port_mapping_toggle)
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
                    checked = mapping?.enabled ?: false,
                    onCheckedChange = { enabled ->
                        if (enabled) permission?.request { notificationGranted = it }
                        problem = plugins.core.send(Action.SetPortMapping(pluginId = installed.id, enabled = enabled))
                    },
                    modifier = Modifier.widthIn(min = WontonTheme.size.target)
                        .semantics { contentDescription = title },
                )
            }
            (mapping?.ports ?: installed.tunnelPorts).forEach { port ->
                WontonSeparator()
                WontonRow(
                    stringResource(
                        R.string.port_mapping_port_line,
                        port.port.toInt(),
                        portProtocolLabel(port.protocol),
                    ),
                )
            }
        }
        mapping?.state?.let { state ->
            portMappingReasonRes(state)?.let { reason ->
                WontonNotice(WontonIcon.Warning, stringResource(reason))
            }
        }
        portMappingNotificationRes(mapping?.enabled == true, notificationGranted)?.let { key ->
            WontonNotice(WontonIcon.Warning, stringResource(key))
        }
        problem?.let { ProblemNotice(it) }
    }
}

/** 端口的协议名；TCP 和 UDP 是协议名，不翻译。 */
@Composable
private fun portProtocolLabel(protocol: PortProtocol): String = when (protocol) {
    PortProtocol.TCP -> "TCP"
    PortProtocol.UDP -> "UDP"
    PortProtocol.OTHER -> stringResource(R.string.port_mapping_protocol_other)
}

/** 映射开着但没有通知权限时的提示：开着也看不到前台服务的通知。其余情形不用提示，返回 null。 */
fun portMappingNotificationRes(enabled: Boolean, notificationGranted: Boolean): Int? = when {
    !enabled || notificationGranted -> null
    else -> R.string.port_mapping_no_notification_permission
}

/** 映射状态的原因文案键；开着和关着不用解释，返回 null。没映射成功时每个类别一句完整的话。 */
fun portMappingReasonRes(state: PortMappingState?): Int? = when (state) {
    null, PortMappingState.ON, PortMappingState.OFF -> null
    PortMappingState.PORT_IN_USE -> R.string.port_mapping_reason_port_in_use
    PortMappingState.SERVER_TOO_OLD -> R.string.port_mapping_reason_server_too_old
    PortMappingState.TUNNEL_DISCONNECTED -> R.string.port_mapping_reason_tunnel_disconnected
    PortMappingState.PLUGIN_NOT_INSTALLED -> R.string.port_mapping_reason_plugin_not_installed
    PortMappingState.PORT_NOT_DECLARED -> R.string.port_mapping_reason_port_not_declared
    PortMappingState.PLUGIN_NOT_RUNNING -> R.string.port_mapping_reason_plugin_not_running
    PortMappingState.PLUGIN_UNREACHABLE -> R.string.port_mapping_reason_plugin_unreachable
    PortMappingState.OTHER -> R.string.port_mapping_reason_other
}
