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
    get() = when (this) {
        is HomeStatus.NotConfigured -> "未配对"
        is HomeStatus.Connected -> "已连接"
        is HomeStatus.Connecting -> ConnectionNotice.Connecting.sentence
        is HomeStatus.Disconnected -> ConnectionNotice.Unreachable(false).sentence
    }

val ConnectionNotice.sentence: String
    get() = when (this) {
        is ConnectionNotice.WaitingForNetwork -> "等待网络…"
        is ConnectionNotice.Connecting -> "连接中…"
        is ConnectionNotice.Updating -> "正在更新…"
        is ConnectionNotice.ServerOffline -> "家里的服务器好像离线了"
        is ConnectionNotice.Unreachable -> "连不上家里的服务器"
        is ConnectionNotice.DeviceRejected -> "这台手机已不被家里的服务器认可"
        is ConnectionNotice.VersionMismatch -> "App 或家里的服务器需要更新"
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
                WontonSmallButton("重新配对", { onOpen(SettingsSubpage.PAIRING) })
            } else {
                notice.retrying?.let { RetryButton(it, onRetry) }
                WontonSmallButton("查看连接", { onOpen(SettingsSubpage.SERVER) })
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
        WontonSmallButton("重试", onRetry, enabled = !retrying)
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
    WontonScreen(bar = { WontonPushedBar("家里的服务器", { onNavigate(SettingsSubpage.MAIN) }) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            ServersGroup(plugins)
            WontonGroup {
                WontonRow("手机网络", details.network.check.text)
                WontonSeparator()
                WontonRow("中继", details.relay.text)
                WontonSeparator()
                WontonRow("家里的服务器", details.server.text)
            }
            viewModel.notice?.retrying?.let { RetryButton(it, onRetry) }
            WontonGroup {
                WontonRow("详情", showsForward = true, onClick = { onNavigate(SettingsSubpage.CONNECTION_DETAILS) })
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
                    "升级服务器",
                    value = "有新版本".takeIf { plugins.viewModel.coreUpgrade.isPrompting() },
                    showsForward = true,
                    onClick = { onNavigate(SettingsSubpage.CORE_UPGRADE) },
                )
            }
            WontonGroup {
                WontonRow("添加设备", showsForward = true, onClick = { onNavigate(SettingsSubpage.INVITE) })
            }
            WontonGroup {
                WontonRow(
                    "重新配对", titleTint = WontonTheme.colors.danger, showsForward = true,
                    onClick = { onNavigate(SettingsSubpage.PAIRING) },
                )
            }
        }
    }
}

@Composable
fun ConnectionDetailsPage(viewModel: HomeViewModel, onBack: () -> Unit) {
    val details = viewModel.details
    WontonScreen(bar = { WontonPushedBar("详情", onBack) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            WontonGroup {
                viewModel.path?.let { path ->
                    WontonRow("路径", path.kindText)
                    WontonSeparator()
                    WontonRow("延迟", "${path.rttMs} ms")
                    WontonSeparator()
                }
                WontonRow("丢包", "${details.lostPackets} / ${details.sentPackets}")
                details.handshakeMs?.let {
                    WontonSeparator()
                    WontonRow("上次握手", "$it ms")
                }
                WontonSeparator()
                WontonRow("断开次数", "${details.disconnects}")
                details.lastConnectedAtMs?.let {
                    WontonSeparator()
                    WontonRow("最后连上", DateFormat.getDateTimeInstance().format(Date(it.toLong())))
                }
            }
            details.lastFailure?.let { WontonGroup { WontonRow("上次失败", it) } }
        }
    }
}

private val HomePath.kindText: String
    get() = when {
        lan -> "局域网"
        relay -> "中继"
        else -> "直连"
    }

/** 连接是否走得通以核心为准，这一行只说系统怎么讲。 */
private val DeviceNetwork?.check: ConnectionCheck
    get() = when (this) {
        null -> ConnectionCheck.UNKNOWN
        DeviceNetwork.NONE -> ConnectionCheck.FAILING
        else -> ConnectionCheck.OK
    }

private val ConnectionCheck.text: String
    get() = when (this) {
        ConnectionCheck.OK -> "正常"
        ConnectionCheck.FAILING -> "不通"
        ConnectionCheck.UNKNOWN -> "不确定"
    }
