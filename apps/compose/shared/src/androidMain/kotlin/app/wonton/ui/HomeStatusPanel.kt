package app.wonton.ui

import android.content.Context
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
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
    val context = LocalContext.current
    WontonGroup {
        WontonRow(
            stringResource(R.string.home_connection),
            connectionText(home),
            valueTint = if (needsTheOwner(home)) colors.danger else null,
            showsForward = true,
            onClick = onConnection,
        )
        WontonSeparator()
        val first = cards.firstOrNull()
        if (first != null) {
            WontonRow(
                stringResource(R.string.home_awaiting),
                pluralStringResource(R.plurals.home_cards, cards.size, cards.size),
                showsForward = true,
                onClick = { onOpenCardRoom(first.roomId) },
            )
        } else {
            WontonRow(stringResource(R.string.home_awaiting), stringResource(R.string.home_none))
        }
        WontonSeparator()
        WontonRow(stringResource(R.string.home_storage), storageText(context, plugins))
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

@Composable
private fun connectionText(home: HomeViewModel): String =
    when (home.status) {
        is HomeStatus.Connected -> home.path?.let {
            val kind = if (it.relay) {
                stringResource(R.string.connection_relay)
            } else {
                stringResource(R.string.connection_direct)
            }
            "${home.status.connectionText} · $kind"
        } ?: home.status.connectionText
        else -> home.notice?.sentence ?: home.status.connectionText
    }

private fun needsTheOwner(home: HomeViewModel): Boolean =
    home.notice is ConnectionNotice.DeviceRejected || home.notice is ConnectionNotice.VersionMismatch

private fun storageText(context: Context, plugins: PluginsViewModel): String =
    plugins.storage?.let {
        context.getString(
            R.string.home_storage_value, sizeText(context, it.freeBytes), sizeText(context, it.totalBytes),
        )
    } ?: context.getString(R.string.egress_path_uncertain)
