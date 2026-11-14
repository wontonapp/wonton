package app.wonton.ui

import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.wonton.protocol.ConnectionCheck
import app.wonton.protocol.ConnectionNotice
import app.wonton.protocol.ConnectionQuality
import app.wonton.protocol.DeviceNetwork
import app.wonton.protocol.HomePath
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.InstalledPlugins
import app.wonton.protocol.UdpFlowStats
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
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.WontonIcon
import java.text.DateFormat
import java.util.Date
import java.util.Locale

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

/** 以核心通知为准，避免 HomeStatus.Connecting 掩盖超时后恢复的重试入口。 */
private val ConnectionNotice.retrying: Boolean?
    get() = when (this) {
        is ConnectionNotice.Connecting -> true
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
    val poor = poorNoticeText(
        home.status is HomeStatus.Connected, notice, home.details.quality, home.path?.relay == true,
    )?.let { stringResource(it) }
    return when {
        poor != null -> WontonBarStatus(title = poor, subtitle = poor)
        // “刚恢复”由核心给：只在连接正常、没有别的提示时为真，到点由核心收掉，壳不计时。
        home.justRecovered -> {
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
    val retrying = notice.retrying == true
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
    // “用 Matrix 账号登录”：入口在下面的服务器列表里，打开后整屏接管，进展来自核心的服务器列表订阅。
    var matrixLoginOpen by rememberSaveable { mutableStateOf(false) }
    var matrixServer by rememberSaveable { mutableStateOf(DEFAULT_MATRIX_SERVER) }
    var dismissedLoginId by remember { mutableStateOf<String?>(null) }
    val login = LocalServers.current.matrixLogin
    // 登录结束（成功、取消或核心清掉）后收起整屏；关掉时记下流程 ID，核心还没处理完取消的那一小段不再弹回来。
    var previousLogin by remember { mutableStateOf(login) }
    LaunchedEffect(login) {
        if (previousLogin != null && (login == null || login.state.isTerminal)) {
            matrixLoginOpen = false
            dismissedLoginId = null
        }
        previousLogin = login
    }
    val flowActive = login != null && !login.state.isTerminal && login.loginId != dismissedLoginId
    if (matrixLoginOpen || flowActive) {
        MatrixLoginScreen(
            login,
            plugins.core,
            matrixServer,
            onServerChange = { matrixServer = it },
            onDismiss = {
                matrixLoginOpen = false
                dismissedLoginId = login?.loginId
            },
        )
        return
    }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.egress_path_home_server), back) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            ServersGroup(plugins) { matrixLoginOpen = true }
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
fun ConnectionDetailsPage(viewModel: HomeViewModel, installed: InstalledPlugins?, onBack: () -> Unit) {
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
            if (details.udpFlows.isNotEmpty()) {
                UdpFlowsGroup(details.udpFlows, details.udpBufferEvictedDatagrams, installed)
            }
            details.lastFailure?.let { WontonGroup { WontonRow(stringResource(R.string.connection_last_failure), it) } }
            details.diagnostic?.let {
                ProblemDetail(it, stringResource(R.string.common_diagnostics))
            }
        }
    }
}

private const val EXPANDED_DEGREES = 90f

/** 隧道上的 UDP 流向（ADR-0072）：一行汇总默认收起，点开每个流向一行；没有流向时整块不出现。 */
@Composable
private fun UdpFlowsGroup(flows: List<UdpFlowStats>, evicted: ULong?, installed: InstalledPlugins?) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    val colors = WontonTheme.colors
    WontonGroup {
        Row(
            Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
                .clickable(role = Role.Button) { expanded = !expanded }
                .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            Text(
                pluralStringResource(R.plurals.connection_udp_flows, flows.size, flows.size),
                style = WontonTheme.typography.body,
                color = colors.textPrimary.toColor(),
            )
            // 测不到（null）和 0 都不显示：iroh 只给连接级发送帧数，核心推不出来时不能伪装成 0。
            Box(Modifier.weight(1f), contentAlignment = Alignment.CenterEnd) {
                evicted?.takeIf { it > 0uL }?.let { dropped ->
                    Text(
                        pluralStringResource(R.plurals.connection_udp_flow_dropped, dropped.toInt(), dropped.toLong()),
                        style = WontonTheme.typography.body.copy(textAlign = TextAlign.End),
                        color = colors.textSecondary.toColor(),
                    )
                }
            }
            Icon(
                WontonIcon.Forward,
                // 装饰：整行可点，箭头只表示能展开。
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconSmall).rotate(if (expanded) EXPANDED_DEGREES else 0f),
                tint = colors.textSecondary.toColor(),
            )
        }
        if (expanded) {
            flows.forEach { flow ->
                WontonSeparator()
                UdpFlowRow(flow, installed)
            }
        }
    }
}

/** 一个流向一行：插件名、端口、收发数；超长丢弃非 0 才显示（ADR-0072）。 */
@Composable
private fun UdpFlowRow(flow: UdpFlowStats, installed: InstalledPlugins?) {
    val title = installed?.plugins?.firstOrNull { it.id == flow.pluginId }?.title ?: flow.pluginId
    val value = if (flow.oversizedDrops > 0uL) {
        stringResource(
            R.string.connection_udp_flow_traffic_dropped,
            flow.port.toInt(),
            flow.receivedPackets.toLong(),
            flow.sentPackets.toLong(),
            flow.oversizedDrops.toLong(),
        )
    } else {
        stringResource(
            R.string.connection_udp_flow_traffic,
            flow.port.toInt(),
            flow.receivedPackets.toLong(),
            flow.sentPackets.toLong(),
        )
    }
    WontonRow(title, value)
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
