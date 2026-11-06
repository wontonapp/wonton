package app.wonton.ui

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import app.wonton.protocol.ConnectionNotice
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PendingConfirmation
import app.wonton.protocol.PluginsViewModel
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 主屏图标下面的状态：连接、等确认的卡片、存储。不知道的如实显示不确定。 */
@Composable
fun HomeStatusPanel(
    home: HomeViewModel,
    plugins: PluginsViewModel,
    cards: List<PendingConfirmation>,
    onConnection: () -> Unit,
    onOpenCardRoom: (String) -> Unit,
) {
    val colors = WontonTheme.colors
    WontonGroup {
        WontonRow(
            "连接",
            connectionText(home),
            valueTint = if (needsTheOwner(home)) colors.danger else null,
            showsForward = true,
            onClick = onConnection,
        )
        WontonSeparator()
        val first = cards.firstOrNull()
        if (first != null) {
            WontonRow("等确认", "${cards.size} 张", showsForward = true, onClick = { onOpenCardRoom(first.roomId) })
        } else {
            WontonRow("等确认", "没有")
        }
        WontonSeparator()
        WontonRow("存储", storageText(plugins))
        plugins.storageProblem?.let { problem ->
            Text(
                problem.message,
                Modifier.fillMaxWidth().padding(
                    start = WontonTheme.spacing.s16,
                    end = WontonTheme.spacing.s16,
                    bottom = WontonTheme.spacing.s12,
                ),
                style = WontonTheme.typography.supporting,
                color = colors.danger.toColor(),
            )
        }
    }
}

private fun connectionText(home: HomeViewModel): String =
    when (home.status) {
        is HomeStatus.Connected -> home.path?.let { "${home.status.connectionText} · ${if (it.relay) "中继" else "直连"}" }
            ?: home.status.connectionText
        else -> home.notice?.sentence ?: home.status.connectionText
    }

private fun needsTheOwner(home: HomeViewModel): Boolean =
    home.notice is ConnectionNotice.DeviceRejected || home.notice is ConnectionNotice.VersionMismatch

private fun storageText(plugins: PluginsViewModel): String =
    plugins.storage?.let { "${sizeText(it.freeBytes)} 可用，共 ${sizeText(it.totalBytes)}" } ?: "不确定"
