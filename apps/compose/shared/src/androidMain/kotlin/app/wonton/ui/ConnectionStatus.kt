package app.wonton.ui

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.ConnectionCheck
import app.wonton.protocol.ConnectionNotice
import app.wonton.protocol.ConnectionQuality
import app.wonton.protocol.DeviceNetwork
import app.wonton.protocol.HomePath
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.ui.design.WontonBarStatus
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.text.DateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.delay

private const val RECOVERY_FEEDBACK_MS = 4_000L

internal val HomeStatus.connectionText: String
    @Composable get() = when (this) {
        is HomeStatus.NotConfigured -> stringResource(R.string.connection_unpaired)
        is HomeStatus.Connected -> stringResource(R.string.connection_connected)
        is HomeStatus.Connecting -> ConnectionNotice.Connecting.sentence
        is HomeStatus.Disconnected -> ConnectionNotice.Unreachable(false).sentence
    }

val ConnectionNotice.sentence: String
    @Composable get() = when (this) {
        is ConnectionNotice.WaitingForNetwork -> stringResource(R.string.connection_waiting_network)
        is ConnectionNotice.Connecting -> stringResource(R.string.connection_connecting)
        is ConnectionNotice.Updating -> stringResource(R.string.connection_updating)
        is ConnectionNotice.ServerOffline -> stringResource(R.string.connection_server_offline)
        is ConnectionNotice.Unreachable -> stringResource(R.string.problem_transport_unavailable_title)
        is ConnectionNotice.DeviceRejected -> stringResource(R.string.connection_device_rejected)
        is ConnectionNotice.VersionMismatch -> stringResource(R.string.connection_version_mismatch)
    }

private val ConnectionNotice.isShort: Boolean
    get() = this is ConnectionNotice.WaitingForNetwork ||
        this is ConnectionNotice.Updating

/** 能重试时给出是否正在重试。 */
private val ConnectionNotice.retrying: Boolean?
    get() = when (this) {
        is ConnectionNotice.ServerOffline -> retrying
        is ConnectionNotice.Unreachable -> retrying
        else -> null
    }

/** 连接 Poor 且连着、没有别的通知时，短状态位置显示的一句；`null` 是不显示（#847）。 */
@StringRes
fun poorNoticeText(
    connected: Boolean,
    notice: ConnectionNotice?,
    quality: ConnectionQuality,
    relayPath: Boolean,
): Int? = when {
    !connected || notice != null || quality != ConnectionQuality.POOR -> null
    relayPath -> R.string.connection_poor_relay
    else -> R.string.connection_poor
}

/** 详情里“连接质量”的取值：Other 也按不确定显示（#847）。 */
@StringRes
fun qualityValueText(quality: ConnectionQuality): Int = when (quality) {
    ConnectionQuality.GOOD -> R.string.connection_quality_good
    ConnectionQuality.POOR -> R.string.connection_quality_poor
    ConnectionQuality.UNKNOWN, ConnectionQuality.OTHER -> R.string.connection_unknown
}

/** 千分比换百分比要除的数：10‰ = 1%。 */
private const val MILLE_PER_PERCENT = 10.0

/** 判定窗口的丢包率：千分比换一位小数的百分比；样本不够时 `null`，界面显示“—”，不能显示 0（#847）。 */
fun lossRateText(lossPerMille: UShort?): String? =
    lossPerMille?.let { String.format(Locale.ROOT, "%.1f%%", it.toDouble() / MILLE_PER_PERCENT) }

/** 判定窗口的往返时间；样本不够时 `null`，界面显示“—”（#847）。 */
fun rttText(rttMs: UInt?): String? = rttMs?.let { "$it ms" }

@Composable
fun connectionBarStatus(
    home: HomeViewModel,
    onRetry: () -> Unit,
    onOpen: (SettingsSubpage) -> Unit,
): WontonBarStatus {
    val notice = home.notice
    var interrupted by remember(home.serverId) { mutableStateOf(false) }
    var recovered by remember(home.serverId) { mutableStateOf(false) }
    LaunchedEffect(home.serverId, home.status, notice != null) {
        if (home.status is HomeStatus.Connected) {
            if (interrupted) {
                recovered = true
                interrupted = false
            }
        } else {
            recovered = false
            if (notice != null) interrupted = true
        }
    }
    LaunchedEffect(recovered) {
        if (recovered) {
            delay(RECOVERY_FEEDBACK_MS)
            recovered = false
        }
    }
    val poor = poorNoticeText(
        home.status is HomeStatus.Connected, notice, home.details.quality, home.path?.relay == true,
    )?.let { stringResource(it) }
    return when {
        poor != null -> WontonBarStatus(title = poor, subtitle = poor)
        recovered && notice == null -> {
            val connected = stringResource(R.string.connection_connected)
            WontonBarStatus(title = connected, subtitle = connected)
        }
        notice == null -> WontonBarStatus()
        notice.isShort -> WontonBarStatus(title = notice.sentence, subtitle = notice.sentence)
        else -> WontonBarStatus(subtitle = notice.sentence, line = { ConnectionLine(notice, onRetry, onOpen) })
    }
}

/** 进度与失败原因互斥，等待结束后由核心重新给出原因和下一步。 */
@Composable
private fun ConnectionLine(notice: ConnectionNotice, onRetry: () -> Unit, onOpen: (SettingsSubpage) -> Unit) {
    val colors = WontonTheme.colors
    val retrying = notice is ConnectionNotice.Connecting || notice.retrying == true
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        if (retrying) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                CircularProgressIndicator(
                    Modifier.size(WontonTheme.size.iconSmall),
                    color = colors.textSecondary.toColor(),
                    strokeWidth = 2.dp,
                )
                Text(
                    stringResource(R.string.connection_retrying),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
            }
        } else {
            Text(
                notice.sentence,
                style = WontonTheme.typography.body,
                color = (if (notice is ConnectionNotice.DeviceRejected) colors.warning else colors.textPrimary)
                    .toColor(),
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            if (notice is ConnectionNotice.DeviceRejected) {
                WontonSmallButton(stringResource(R.string.connection_pair_again), { onOpen(SettingsSubpage.PAIRING) })
            } else {
                if (notice.retrying == false) RetryButton(false, onRetry)
                WontonSmallButton(stringResource(R.string.connection_view), { onOpen(SettingsSubpage.SERVER) })
            }
        }
    }
}

@Composable
private fun RetryButton(retrying: Boolean, onRetry: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        if (retrying) {
            CircularProgressIndicator(
                Modifier.size(WontonTheme.size.iconSmall),
                color = WontonTheme.colors.textSecondary.toColor(),
                strokeWidth = 2.dp,
            )
        }
        WontonSmallButton(stringResource(R.string.common_retry), onRetry, enabled = !retrying)
    }
}

/** 设置里的家用服务器：连接诊断和服务器管理入口。 */
@Composable
fun ConnectionPage(
    viewModel: HomeViewModel,
    plugins: PluginsState,
    onRetry: () -> Unit,
    onNavigate: (SettingsSubpage) -> Unit,
) {
    val details = viewModel.details
    val back = { onNavigate(SettingsSubpage.MAIN) }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.egress_path_home_server), back) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            ServersGroup(plugins)
            WontonGroup {
                WontonRow(stringResource(R.string.connection_mobile_network), details.network.check.text)
                WontonSeparator()
                WontonRow(stringResource(R.string.connection_relay), details.relay.text)
                WontonSeparator()
                WontonRow(stringResource(R.string.egress_path_home_server), details.server.text)
            }
            viewModel.notice?.retrying?.let { RetryButton(it, onRetry) }
            WontonGroup {
                WontonRow(stringResource(R.string.connection_details_title), showsForward = true) {
                    onNavigate(SettingsSubpage.CONNECTION_DETAILS)
                }
            }
            WontonGroup {
                WontonRow(
                    stringResource(R.string.egress_title),
                    showsForward = true,
                    onClick = { onNavigate(SettingsSubpage.EGRESS) },
                )
                WontonSeparator()
                WontonRow(stringResource(R.string.registry_mirror_title), showsForward = true) {
                    onNavigate(SettingsSubpage.REGISTRY_MIRROR)
                }
                WontonSeparator()
                WontonRow(
                    stringResource(R.string.core_upgrade_title),
                    value = stringResource(R.string.connection_new_version)
                        .takeIf { plugins.viewModel.coreUpgrade.isPrompting() },
                    showsForward = true,
                    onClick = { onNavigate(SettingsSubpage.CORE_UPGRADE) },
                )
            }
            WontonGroup {
                WontonRow(stringResource(R.string.connection_add_device), showsForward = true) {
                    onNavigate(SettingsSubpage.INVITE)
                }
            }
            WontonGroup {
                WontonRow(
                    stringResource(R.string.connection_pair_again), titleTint = WontonTheme.colors.danger,
                    showsForward = true, onClick = { onNavigate(SettingsSubpage.PAIRING) },
                )
            }
        }
    }
}

@Composable
fun ConnectionDetailsPage(viewModel: HomeViewModel, onBack: () -> Unit) {
    val details = viewModel.details
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.connection_details_title), onBack) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            WontonGroup {
                if (details.addressesStale) {
                    WontonRow(stringResource(R.string.connection_addresses_stale))
                    WontonSeparator()
                }
                viewModel.path?.let { path ->
                    WontonRow(stringResource(R.string.connection_path), path.kindText)
                    WontonSeparator()
                }
                WontonRow(
                    stringResource(R.string.connection_quality),
                    stringResource(qualityValueText(details.quality)),
                )
                WontonSeparator()
                WontonRow(
                    stringResource(R.string.connection_loss_rate),
                    lossRateText(details.lossPerMille) ?: stringResource(R.string.connection_value_none),
                )
                WontonSeparator()
                WontonRow(
                    stringResource(R.string.connection_latency),
                    rttText(details.rttMs) ?: stringResource(R.string.connection_value_none),
                )
                details.handshakeMs?.let {
                    WontonSeparator()
                    WontonRow(stringResource(R.string.connection_last_handshake), "$it ms")
                }
                WontonSeparator()
                WontonRow(stringResource(R.string.connection_disconnects), "${details.disconnects}")
                details.lastConnectedAtMs?.let {
                    WontonSeparator()
                    WontonRow(
                        stringResource(R.string.connection_last_connected),
                        DateFormat.getDateTimeInstance().format(Date(it.toLong())),
                    )
                }
            }
            details.lastFailure?.let { WontonGroup { WontonRow(stringResource(R.string.connection_last_failure), it) } }
        }
    }
}

private val HomePath.kindText: String
    @Composable get() = when {
        lan -> stringResource(R.string.connection_lan)
        relay -> stringResource(R.string.connection_relay)
        else -> stringResource(R.string.connection_direct)
    }

/** 连接是否走得通以核心为准，这一行只说系统怎么讲。 */
private val DeviceNetwork?.check: ConnectionCheck
    get() = when (this) {
        null -> ConnectionCheck.UNKNOWN
        DeviceNetwork.NONE -> ConnectionCheck.FAILING
        else -> ConnectionCheck.OK
    }

private val ConnectionCheck.text: String
    @Composable get() = when (this) {
        ConnectionCheck.OK -> stringResource(R.string.connection_ok)
        ConnectionCheck.FAILING -> stringResource(R.string.connection_failing)
        ConnectionCheck.UNKNOWN -> stringResource(R.string.connection_unknown)
    }
