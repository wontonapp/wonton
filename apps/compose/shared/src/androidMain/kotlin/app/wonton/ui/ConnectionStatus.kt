package app.wonton.ui

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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.ConnectionCheck
import app.wonton.protocol.ConnectionNotice
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
        this is ConnectionNotice.Connecting ||
        this is ConnectionNotice.Updating

/** 能重试时给出是否正在重试。 */
private val ConnectionNotice.retrying: Boolean?
    get() = when (this) {
        is ConnectionNotice.ServerOffline -> retrying
        is ConnectionNotice.Unreachable -> retrying
        else -> null
    }

@Composable
fun connectionBarStatus(
    notice: ConnectionNotice?,
    onRetry: () -> Unit,
    onOpen: (SettingsSubpage) -> Unit,
): WontonBarStatus {
    return when {
        notice == null -> WontonBarStatus()
        notice.isShort -> WontonBarStatus(title = notice.sentence, subtitle = notice.sentence)
        else -> WontonBarStatus(subtitle = notice.sentence, line = { ConnectionLine(notice, onRetry, onOpen) })
    }
}

/** 长时间连不上：一句话和下一步。不用红色：还在自动重试，也不是主人做错了什么；要主人动手的用警告色。 */
@Composable
private fun ConnectionLine(notice: ConnectionNotice, onRetry: () -> Unit, onOpen: (SettingsSubpage) -> Unit) {
    val colors = WontonTheme.colors
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(
            notice.sentence,
            style = WontonTheme.typography.body,
            color = (if (notice is ConnectionNotice.DeviceRejected) colors.warning else colors.textPrimary).toColor(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            if (notice is ConnectionNotice.DeviceRejected) {
                WontonSmallButton(stringResource(R.string.connection_pair_again), { onOpen(SettingsSubpage.PAIRING) })
            } else {
                notice.retrying?.let { RetryButton(it, onRetry) }
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
                    WontonRow(stringResource(R.string.connection_latency), "${path.rttMs} ms")
                    WontonSeparator()
                }
                WontonRow(
                    stringResource(R.string.connection_lost_packets),
                    "${details.lostPackets} / ${details.sentPackets}",
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
