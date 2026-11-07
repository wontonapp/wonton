package app.wonton.ui

import android.icu.text.DateFormat
import android.icu.text.RelativeDateTimeFormatter
import android.icu.text.RelativeDateTimeFormatter.RelativeUnit
import android.icu.util.ULocale
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.PeerError
import app.wonton.protocol.PeerRow
import app.wonton.protocol.PeerStatus
import app.wonton.protocol.PeersState
import app.wonton.protocol.PeersViewModel
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Expand
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.QrCode
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon
import java.util.Date
import kotlin.math.roundToInt

private const val SECOND_MS = 1000L
private const val SECONDS_PER_MINUTE = 60
private const val SECONDS_PER_HOUR = 60.0 * SECONDS_PER_MINUTE
private const val SECONDS_PER_DAY = SECONDS_PER_HOUR * 24
private const val SECONDS_PER_MONTH = SECONDS_PER_DAY * 30

/**
 * “其他服务器”（ADR-0050）：和别的 Wonton 服务器配对。列表、发来的请求、出的码和失败原因都来自核心的
 * [ClientCore.peersUpdates]；进页面取一次，之后下拉刷新。扫码失败的重试是回到扫码，其余的重取列表。
 * 配对提醒的 App 链接带着“到请求”来，[openRequests] 为真时滚回顶部。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun PeersPage(
    core: ClientCore,
    onBack: () -> Unit,
    openRequests: Boolean = false,
    onOpenRequestsHandled: () -> Unit = {},
) {
    val peers by remember(core) { core.peersUpdates() }.collectAsState(remember(core) { core.peers() })
    var adding by remember { mutableStateOf(false) }
    var scanning by remember { mutableStateOf(false) }
    var showingCode by remember { mutableStateOf(false) }
    var removing by remember { mutableStateOf<PeerRow?>(null) }
    var rejection by remember { mutableStateOf<Problem?>(null) }
    // 最近一次是扫码失败还是取列表失败，决定“重试”回到哪边。
    var lastScan by remember { mutableStateOf(false) }
    val scroll = rememberScrollState()
    val send = { action: Action -> rejection = core.send(action) }
    LaunchedEffect(core) { send(Action.RefreshPeers) }
    // 出码页开着时核心才快取列表；进和出都告诉它（ADR-0050）。
    val showCode = {
        showingCode = true
        send(Action.ShowPeerCode)
    }
    val hideCode = {
        showingCode = false
        send(Action.HidePeerCode)
    }
    // 请求在别的设备上处理过了：提醒一句，重取列表让它消失。
    LaunchedEffect(peers.state) {
        if ((peers.state as? PeersState.Failed)?.error == PeerError.REQUEST_NOT_FOUND) send(Action.RefreshPeers)
    }
    LaunchedEffect(openRequests) {
        if (openRequests) {
            scroll.animateScrollTo(0)
            onOpenRequestsHandled()
        }
    }
    val refresh = {
        lastScan = false
        send(Action.RefreshPeers)
    }
    val retry = { if (lastScan) scanning = true else refresh() }
    val scanSubmitted = { code: String ->
        lastScan = true
        send(Action.ScanPeerCode(code))
    }
    // 取消自己发出、对方还没处理的请求，清掉被拒的记录，都直接做；移除配对两边都会停，才要确认。
    val requestRemove: (PeerRow) -> Unit = { row ->
        if (row.removesDirectly()) {
            send(Action.RemovePeer(row.serverName))
        } else {
            removing = row
        }
    }
    if (scanning) {
        PeerScanPage(
            core,
            busy = peers.state is PeersState.Loading,
            onBack = { scanning = false },
            onScan = {
                scanning = false
                scanSubmitted(it)
            },
            onPasteSubmit = scanSubmitted,
        )
    } else {
        WontonScreen(bar = {
            WontonPushedBar(stringResource(R.string.peers_title), onBack, actions = {
                WontonIconButton(WontonIcon.Add, stringResource(R.string.peers_add), { adding = true })
            })
        }) {
            PullToRefreshBox(
                isRefreshing = peers.state is PeersState.Loading && peers.peers.isNotEmpty(),
                onRefresh = refresh,
                modifier = Modifier.fillMaxSize(),
            ) {
                val state = peers.state
                // 页面上有没有内容看三份：请求、配对过的服务器、已拒绝。有内容时失败都降成列表顶部的一条。
                val hasRows = peers.peers.isNotEmpty() || peers.requests.isNotEmpty() || peers.rejected.isNotEmpty()
                when {
                    hasRows -> PeerList(peers, scroll, retry, requestRemove, send)
                    state is PeersState.Failed ->
                        ProblemView(state.problem(), retry.takeIf { state.error.retriesHelpfully() })
                    state == PeersState.Loading -> LoadingView()
                    else -> PeerEmpty(onShow = showCode, onScan = { scanning = true })
                }
            }
        }
    }
    if (adding) {
        AddServerSheet(
            onShow = showCode,
            onScan = { scanning = true },
            onDismiss = { adding = false },
        )
    }
    if (showingCode) PeerCodeDialog(peers.code, onDismiss = hideCode, onReset = { send(Action.ResetPeerCode) })
    removing?.let { row ->
        RemovePeerDialog(row, onDismiss = { removing = null }, onConfirm = {
            removing = null
            send(Action.RemovePeer(row.serverName))
        })
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

/** 加一台服务器：出我的码给对方扫，或去扫对方的码。 */
@Composable
private fun AddServerSheet(onShow: () -> Unit, onScan: () -> Unit, onDismiss: () -> Unit) {
    WontonActionSheet(
        listOf(
            WontonSheetAction(stringResource(R.string.peers_show_code), WontonIcon.QrCode, onClick = onShow),
            WontonSheetAction(stringResource(R.string.peers_scan_code), WontonIcon.Expand, onClick = onScan),
        ),
        onDismiss = onDismiss,
    )
}

/** 移除一台配对的服务器的确认框：写法照 Servers.kt，标题带名字，按钮用 common_remove。 */
@Composable
private fun RemovePeerDialog(row: PeerRow, onDismiss: () -> Unit, onConfirm: () -> Unit) {
    WontonDialog(
        stringResource(R.string.peers_remove_title, row.ownerDisplayName),
        stringResource(R.string.peers_remove_message),
        onDismiss,
        listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(stringResource(R.string.common_remove), WontonButtonKind.DANGER, onConfirm),
        ),
    )
}

/** 配对接口失败的一句话，按核心给的类别写（ADR-0043）；核心按 App 语言写好的说明收进“详情”。 */
internal fun PeersState.Failed.problem() = Problem(
    titleRes = when (error) {
        PeerError.CODE_INVALID -> R.string.peers_error_code_invalid
        PeerError.CODE_REVOKED -> R.string.peers_error_code_revoked
        PeerError.TOO_MANY_REQUESTS -> R.string.peers_error_too_many_requests
        PeerError.REQUEST_NOT_FOUND -> R.string.peers_error_request_not_found
        PeerError.PEER_UNREACHABLE -> R.string.peers_error_peer_unreachable
        PeerError.PEER_REJECTED -> R.string.peers_error_peer_rejected
        PeerError.ALREADY_PAIRED -> R.string.peers_error_already_paired
        PeerError.SELF_PAIRING -> R.string.peers_error_self_pairing
        PeerError.NOT_PAIRED -> R.string.peers_error_not_paired
        PeerError.UNAUTHENTICATED -> R.string.peers_error_unauthenticated
        PeerError.TRANSPORT_UNAVAILABLE -> R.string.peers_error_transport
        PeerError.PROTOCOL, PeerError.OTHER -> R.string.peers_error_other
    },
    detail = message,
)

/** 还没有配对过的服务器：一句话和两个添加的办法。 */
@Composable
private fun PeerEmpty(onShow: () -> Unit, onScan: () -> Unit) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.peers_empty),
            style = WontonTheme.typography.body.copy(textAlign = TextAlign.Center),
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonButton(
                stringResource(R.string.peers_show_code), onShow,
                Modifier.weight(1f), WontonButtonKind.SECONDARY,
            )
            WontonButton(
                stringResource(R.string.peers_scan_code), onScan,
                Modifier.weight(1f), WontonButtonKind.SECONDARY,
            )
        }
    }
}

/** 列表：发来的请求在上面，往下是配对过的服务器，最底下默认收起的“已拒绝”。失败和刷新中的提示都让开列表的位置。 */
@Composable
private fun PeerList(
    peers: PeersViewModel,
    scroll: ScrollState,
    onRetry: () -> Unit,
    onRemove: (PeerRow) -> Unit,
    onAction: (Action) -> Unit,
) {
    Column(
        Modifier.fillMaxSize().verticalScroll(scroll)
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        val state = peers.state
        if (state is PeersState.Failed) {
            if (state.error == PeerError.REQUEST_NOT_FOUND) ProblemNotice(state.problem(), WontonIcon.Info)
            else ProblemRow(state.problem(), onRetry.takeIf { state.error.retriesHelpfully() })
        }
        if (peers.requests.isNotEmpty()) {
            PeerSection {
                Text(
                    stringResource(R.string.peers_requests_title),
                    style = WontonTheme.typography.sectionTitle,
                    color = WontonTheme.colors.textPrimary.toColor(),
                )
                WontonGroup {
                    peers.requests.forEachIndexed { index, row ->
                        if (index > 0) WontonSeparator()
                        PeerRequestRowView(
                            row,
                            onApprove = { onAction(Action.ApprovePeerRequest(row.id)) },
                            onReject = { onAction(Action.RejectPeerRequest(row.id)) },
                        )
                    }
                }
            }
        }
        if (peers.peers.isNotEmpty()) {
            WontonGroup {
                peers.peers.forEachIndexed { index, row ->
                    if (index > 0) WontonSeparator()
                    PeerRowView(row, onRemove)
                }
            }
        }
        if (peers.rejected.isNotEmpty()) {
            RejectedSection(peers.rejected, onUnreject = { onAction(Action.UnrejectPeerRequest(it)) })
        }
    }
}

/** 取消请求、清掉被拒的记录这两类移除不用确认；移除配对两边都会停，才要。 */
private fun PeerRow.removesDirectly() = status == PeerStatus.AWAITING_APPROVAL || status == PeerStatus.DECLINED

/** 一台对方服务器：标题是对方主人的显示名，下面是状态；点开展开服务器名和详情，长按或行尾删除。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun PeerRowView(row: PeerRow, onRemove: (PeerRow) -> Unit) {
    var expanded by remember(row.serverName) { mutableStateOf(false) }
    var menu by remember(row.serverName) { mutableStateOf(false) }
    val colors = WontonTheme.colors
    val secondary = colors.textSecondary.toColor()
    Column(
        Modifier.fillMaxWidth().combinedClickable(
            role = Role.Button,
            onClick = { expanded = !expanded },
            onLongClick = { menu = true },
            onLongClickLabel = stringResource(R.string.common_remove),
        ).padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(row.ownerDisplayName, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
                PeerSubtitle(row)
            }
            // 对方已经断开、对方拒绝了、请求还没被处理：这一行留着碍事，把移除直接摆在行尾。
            if (row.status == PeerStatus.REMOVED_BY_PEER || row.status == PeerStatus.DECLINED ||
                row.status == PeerStatus.AWAITING_APPROVAL
            ) {
                WontonSmallButton(stringResource(R.string.common_remove), { onRemove(row) }, danger = true)
            }
        }
        if (expanded) {
            Text(
                row.serverName,
                style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                color = secondary,
            )
            row.detail?.let { Text(it, style = WontonTheme.typography.supporting, color = secondary) }
        }
    }
    if (menu) {
        WontonActionSheet(
            listOf(
                WontonSheetAction(stringResource(R.string.common_remove), WontonIcon.Trash, isDanger = true) {
                    onRemove(row)
                },
            ),
            onDismiss = { menu = false },
            title = row.ownerDisplayName,
        )
    }
}

/** 状态行：连上时什么都不写（ADR-0050）。 */
@Composable
private fun PeerSubtitle(row: PeerRow) {
    val style = WontonTheme.typography.supporting
    val color = WontonTheme.colors.textSecondary.toColor()
    when (row.status) {
        PeerStatus.CONNECTED -> Unit
        PeerStatus.OFFLINE -> {
            Text(stringResource(R.string.peers_status_offline), style = style, color = color)
            row.lastConnectedMs?.let {
                Text(
                    stringResource(R.string.peers_last_connected, RelativeTime(it.toLong())),
                    style = style, color = color,
                )
            }
        }
        PeerStatus.REMOVED_BY_PEER -> Text(
            stringResource(R.string.peers_status_removed), style = style, color = color,
        )
        PeerStatus.CHECKING -> Text(
            stringResource(R.string.peers_status_checking), style = style, color = color,
        )
        PeerStatus.AWAITING_APPROVAL -> Text(
            stringResource(R.string.peers_status_awaiting), style = style, color = color,
        )
        PeerStatus.DECLINED -> Text(
            stringResource(R.string.peers_status_declined), style = style, color = color,
        )
        PeerStatus.OTHER -> Text(stringResource(R.string.peers_status_unknown), style = style, color = color)
    }
}

/** 相对时间（“3 小时前”），按 App 当前的语言格式化（ADR-0043）；一分钟内是“刚刚”，超过一个月用日期。 */
@Composable
internal fun RelativeTime(ms: Long): String {
    val locale = LocalConfiguration.current.locales[0]
    val formatter = remember(locale) { RelativeDateTimeFormatter.getInstance(ULocale.forLocale(locale)) }
    val dateFormatter = remember(locale) { DateFormat.getDateInstance(DateFormat.SHORT, locale) }
    val seconds = ((System.currentTimeMillis() - ms) / SECOND_MS.toDouble()).coerceAtLeast(0.0)
    return when {
        seconds < SECONDS_PER_MINUTE -> stringResource(R.string.peers_just_now)
        seconds < SECONDS_PER_MONTH -> {
            // Android 的 RelativeDateTimeFormatter 没有按数量自动挑单位的两参 format，这里按数量级挑。
            val (value, unit) = when {
                seconds < SECONDS_PER_HOUR -> rounded(seconds, SECONDS_PER_MINUTE.toDouble()) to RelativeUnit.MINUTES
                seconds < SECONDS_PER_DAY -> rounded(seconds, SECONDS_PER_HOUR) to RelativeUnit.HOURS
                else -> rounded(seconds, SECONDS_PER_DAY) to RelativeUnit.DAYS
            }
            formatter.format(value, RelativeDateTimeFormatter.Direction.LAST, unit)
        }
        else -> dateFormatter.format(Date(ms))
    }
}

/** 按单位折算成整数（至少 1），ICU 的相对格式带小数不好看。 */
internal fun rounded(seconds: Double, divisor: Double) =
    (seconds / divisor).roundToInt().coerceAtLeast(1).toDouble()
