// 截图测试的假 ViewModel 数据和预览都在测试源集里，不进 App 包；假核心给的原文就是写死的字面量，
// 预览多了也超过了 detekt 每文件 11 个函数的上限。`Notices`、`InviteItem` 为此外提成 internal。
@file:Suppress("MagicNumber", "TooManyFunctions")

package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.ChatAccount
import app.wonton.protocol.ChatFailure
import app.wonton.protocol.ChatForm
import app.wonton.protocol.ChatLayout
import app.wonton.protocol.ChatLayoutChoice
import app.wonton.protocol.ChatSync
import app.wonton.protocol.ChatTextScale
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.CodeOverflow
import app.wonton.protocol.ConnectionCheck
import app.wonton.protocol.ConnectionDetails
import app.wonton.protocol.ConnectionQuality
import app.wonton.protocol.CoreUpgradeStage
import app.wonton.protocol.CoreUpgradeViewModel
import app.wonton.protocol.DeviceNetwork
import app.wonton.protocol.DeviceVerification
import app.wonton.protocol.EgressViewModel
import app.wonton.protocol.EncryptionStatus
import app.wonton.protocol.FontStyle
import app.wonton.protocol.HomePath
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.InstalledPlugins
import app.wonton.protocol.PendingConfirmation
import app.wonton.protocol.PluginKind
import app.wonton.protocol.PluginRuntime
import app.wonton.protocol.PluginState
import app.wonton.protocol.PluginTrust
import app.wonton.protocol.PluginsViewModel
import app.wonton.protocol.PushStatus
import app.wonton.protocol.RecoveryNextStep
import app.wonton.protocol.RecoveryStatus
import app.wonton.protocol.RegistryMirrorViewModel
import app.wonton.protocol.RolesViewModel
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.protocol.Rgb
import app.wonton.protocol.SidebarViewModel
import app.wonton.protocol.StorageUsage
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
import app.wonton.protocol.UdpFlowStats
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 预览和截图测试的固定数据：按接口给的字段直接构造，不经过核心，JVM 上也不用装原生库。
 * 只写看得见的那部分，其余取空值，好让画面里的每一行都在参考图上出现。
 */

private fun rgb(hex: Int) = Rgb(
    (hex shr 16 and 0xFF).toUByte(),
    (hex shr 8 and 0xFF).toUByte(),
    (hex and 0xFF).toUByte(),
)

private fun palette(dark: Boolean) = if (dark) {
    ThemeColors(
        canvas = rgb(0x000000),
        surface = rgb(0x1C1C1E),
        surfaceSubtle = rgb(0x2C2C2E),
        textPrimary = rgb(0xF2F2F7),
        textSecondary = rgb(0x8E8E93),
        separator = rgb(0x38383A),
        primary = rgb(0xFFFFFF),
        onPrimary = rgb(0x000000),
        success = rgb(0x30D158),
        warning = rgb(0xFFD60A),
        danger = rgb(0xFF453A),
        accentFill = rgb(0x0A2540),
        accentInk = rgb(0x64B5F6),
        accentInkOnFill = rgb(0xCDE7FF),
        textOnAccentFill = rgb(0xE3F2FD),
        accentFillBorder = rgb(0x1E4976),
    )
} else {
    ThemeColors(
        canvas = rgb(0xFFFFFF),
        surface = rgb(0xFFFFFF),
        surfaceSubtle = rgb(0xF2F2F2),
        textPrimary = rgb(0x000000),
        textSecondary = rgb(0x666666),
        separator = rgb(0xDDDDDD),
        primary = rgb(0x000000),
        onPrimary = rgb(0xFFFFFF),
        success = rgb(0x2E7D32),
        warning = rgb(0xF9A825),
        danger = rgb(0xC62828),
        accentFill = rgb(0xE3F2FD),
        accentInk = rgb(0x1565C0),
        accentInkOnFill = rgb(0x0D47A1),
        textOnAccentFill = rgb(0x0D47A1),
        accentFillBorder = rgb(0xBBDEFB),
    )
}

internal fun previewAppearance(dark: Boolean = false) = AppearanceViewModel(
    accent = rgb(0x3366FF),
    presets = listOf(rgb(0x3366FF)),
    moreColors = emptyList(),
    swatchColumns = 6u,
    themeMode = if (dark) ThemeMode.DARK else ThemeMode.LIGHT,
    themeModes = listOf(ThemeMode.SYSTEM, ThemeMode.LIGHT, ThemeMode.DARK),
    textSize = TextSize.STANDARD,
    textSizes = listOf(TextSize.STANDARD),
    font = FontStyle.DEFAULT,
    fonts = listOf(FontStyle.DEFAULT),
    light = palette(dark = false),
    dark = palette(dark = true),
    problem = null,
)

private const val DEVICE_ID = "6f1d3c47a90b5e28d34c1a7b0e965f2c8d4a13b6e075c9f2a8b4d61e3c05f7a9"
private const val SERVER_ID = "b71e5048c29a6d3f"

internal fun previewHome() = HomeViewModel(
    deviceId = DEVICE_ID,
    serverId = SERVER_ID,
    status = HomeStatus.Connected,
    path = HomePath(relay = true, publicRelay = false, lan = false, rttMs = 48u),
    notice = null,
    details = ConnectionDetails(
        network = DeviceNetwork.WIFI,
        relay = ConnectionCheck.OK,
        server = ConnectionCheck.OK,
        lastConnectedAtMs = 1_762_800_000_000u,
        handshakeMs = 180u,
        disconnects = 1u,
        failedDials = 0u,
        sentPackets = 128_400u,
        lostPackets = 32u,
        quality = ConnectionQuality.GOOD,
        lossPerMille = 2u,
        rttMs = 48u,
        lastFailure = null,
        addressesStale = false,
        udpFlows = listOf(
            UdpFlowStats(
                flowId = 3u,
                pluginId = "app.sunshine",
                port = 47998u,
                receivedPackets = 1_824uL,
                sentPackets = 1_795uL,
                oversizedDrops = 3uL,
                bufferEvictedDrops = null,
            ),
            UdpFlowStats(
                flowId = 4u,
                pluginId = "app.neko",
                port = 8080u,
                receivedPackets = 412uL,
                sentPackets = 398uL,
                oversizedDrops = 0uL,
                bufferEvictedDrops = null,
            ),
        ),
        udpBufferEvictedDatagrams = 57uL,
    ),
    proxy = null,
    webUrl = null,
    pairing = null,
    invite = null,
    keepConnection = false,
    justRecovered = false,
)

/** 连接详情预览里 UDP 流向指向的已装插件：只填画面用得到的名字，其余取空值。 */
internal fun previewInstalled() = InstalledPlugins(
    plugins = listOf(
        InstalledPlugin(
            id = "app.sunshine",
            title = "Sunshine",
            summary = "",
            version = "1.0.0",
            revision = "r1",
            kind = PluginKind.APP,
            runtime = PluginRuntime.COMPOSE,
            icon = null,
            trust = PluginTrust.COMMUNITY,
            source = null,
            provider = null,
            viaIndex = null,
            writtenBy = null,
            status = PluginState.Stopped,
            webUrl = null,
            crossOriginIsolated = false,
            declaresTools = false,
            tabs = emptyList(),
            chat = null,
            botUserId = null,
            family = false,
            upgrade = null,
            removable = true,
            notices = emptyList(),
            enabledRootPermissions = emptyList(),
            installProgress = null,
            installFailure = null,
            tunnelPorts = emptyList(),
        ),
        InstalledPlugin(
            id = "app.neko",
            title = "neko",
            summary = "",
            version = "1.0.0",
            revision = "r1",
            kind = PluginKind.APP,
            runtime = PluginRuntime.COMPOSE,
            icon = null,
            trust = PluginTrust.COMMUNITY,
            source = null,
            provider = null,
            viaIndex = null,
            writtenBy = null,
            status = PluginState.Stopped,
            webUrl = null,
            crossOriginIsolated = false,
            declaresTools = false,
            tabs = emptyList(),
            chat = null,
            botUserId = null,
            family = false,
            upgrade = null,
            removable = true,
            notices = emptyList(),
            enabledRootPermissions = emptyList(),
            installProgress = null,
            installFailure = null,
            tunnelPorts = emptyList(),
        ),
    ),
    upgradeCheck = null,
)

internal fun previewPlugins() = PluginsViewModel(
    installed = InstalledPlugins(plugins = emptyList(), upgradeCheck = null),
    store = null,
    sidebar = SidebarViewModel(apps = emptyList(), moreApps = false, updates = 0u, settingsBadge = false),
    refreshing = false,
    problem = null,
    tasks = emptyList(),
    updatable = emptyList(),
    pending = emptyList(),
    confirmation = null,
    egress = EgressViewModel(
        current = null,
        loading = false,
        saving = false,
        confirmation = null,
        loadProblem = null,
        saveProblem = null,
    ),
    registryMirror = RegistryMirrorViewModel(
        settings = null,
        loading = false,
        saving = false,
        testing = false,
        loadProblem = null,
        saveProblem = null,
        testProblem = null,
        tests = null,
    ),
    coreUpgrade = CoreUpgradeViewModel(
        status = null,
        stage = CoreUpgradeStage.Idle,
        offer = null,
        compatDetails = emptyList(),
        offerSkipped = false,
        refreshing = false,
        problem = null,
    ),
    storage = StorageUsage(totalBytes = 512_000_000_000u, freeBytes = 380_000_000_000u),
    storageProblem = null,
    takeover = null,
    actions = emptyList(),
    sharedApps = null,
    portMappings = emptyList(),
    ownerSecrets = null,
    mediaGrants = emptyList(),
    roles = RolesViewModel(loading = false, roles = emptyList(), defaultInviteRole = null, problem = null),
)

internal fun previewCard() = PendingConfirmation(
    id = "card-1",
    roomId = "!hermes:home.example",
    title = "Hermes wants to install File Search",
)

internal fun previewEncryption() = EncryptionStatus(
    verification = DeviceVerification.UNKNOWN,
    recovery = RecoveryStatus.DISABLED,
    recoveryKey = null,
    backupReminder = null,
    canVerifyWithOtherDevice = false,
    canReset = false,
    recoveryNextStep = RecoveryNextStep.NONE,
    verificationFlow = null,
)

internal fun previewChat() = ChatViewModel(
    account = ChatAccount.SignedOut,
    sync = ChatSync.Idle,
    encryption = previewEncryption(),
    installingBots = emptyList(),
    rooms = emptyList(),
    problem = null,
    push = PushStatus.NoEndpoint,
    pendingConfirmations = emptyList(),
    settledConfirmations = emptyList(),
    backgroundNotifications = emptyList(),
    bots = emptyList(),
    chatLayoutDefault = ChatLayoutChoice(form = null, textScale = null, code = null),
)

/**
 * 截图测试和 IDE 预览的起点：固定浅色、英文、手机宽度。预览自己带主题，测试不再套一层，
 * 免得两处各写一套主题而比出没意义的差异。
 */
@Composable
internal fun PreviewFrame(dark: Boolean = false, content: @Composable () -> Unit) {
    WontonTheme(previewAppearance(dark)) {
        Box(Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())) {
            content()
        }
    }
}

/** 主屏图标下面那组状态：连接、等答案的卡片、存储（首屏）。 */
@Preview(name = "首屏状态", locale = "en", widthDp = 390, heightDp = 200)
@Composable
internal fun HomeStatusPanelPreview() {
    PreviewFrame {
        HomeStatusPanel(
            home = previewHome(),
            plugins = previewPlugins(),
            cards = listOf(previewCard()),
            onConnection = {},
            onOpenCardRoom = {},
        )
    }
}

/** 深色：同一组状态换个主题，颜色表配错时参考图会变。 */
@Preview(name = "首屏状态（深色）", locale = "en", widthDp = 390, heightDp = 200)
@Composable
internal fun HomeStatusPanelDarkPreview() {
    PreviewFrame(dark = true) {
        HomeStatusPanel(
            home = previewHome(),
            plugins = previewPlugins(),
            cards = listOf(previewCard()),
            onConnection = {},
            onOpenCardRoom = {},
        )
    }
}

/** 连接详情页（ADR-0072 的 UDP 流向）：汇总行默认收起，点开每个流向一行。 */
@Preview(name = "连接详情", locale = "en", widthDp = 390, heightDp = 560)
@Composable
internal fun ConnectionDetailsFlowsPreview() {
    PreviewFrame {
        ConnectionDetailsPage(previewHome(), previewInstalled(), onBack = {})
    }
}

/** 列表里的邀请行，预览用；只填看得见的字段。 */
internal fun previewInviteRow() = RoomRow(
    roomId = "!design:home.example",
    name = "Design review",
    isDirect = false,
    directUserId = null,
    isEncrypted = true,
    isReadOnly = false,
    membership = RoomMembership.INVITED,
    preview = null,
    sideChats = false,
    unreadCount = 0u,
    layout = ChatLayout(form = ChatForm.BUBBLES, textScale = ChatTextScale.STANDARD, code = CodeOverflow.WRAP),
)

/** 聊天列表顶部的状态提示（聊天列表）。 */
@Preview(name = "聊天提示", locale = "en", widthDp = 390, heightDp = 160)
@Composable
internal fun ChatNoticesPreview() {
    PreviewFrame {
        Notices(viewModel = previewChat(), onOpenBackup = {})
    }
}

/** 聊天列表里的失败提示：说清原因、给下一步、详情默认收起。 */
@Preview(name = "聊天失败提示", locale = "en", widthDp = 390, heightDp = 220)
@Composable
internal fun ChatFailurePreview() {
    PreviewFrame {
        Notices(
            viewModel = previewChat().copy(
                account = ChatAccount.Failed(reason = ChatFailure.UPSTREAM, message = "502 Bad Gateway"),
            ),
            onOpenBackup = {},
        )
    }
}

/** 有人邀请进房间（聊天列表）。 */
@Preview(name = "聊天邀请", locale = "en", widthDp = 390, heightDp = 160)
@Composable
internal fun ChatInvitePreview() {
    PreviewFrame {
        InviteItem(row = previewInviteRow(), onAction = {})
    }
}

/** 整页都显示不出来时的样子（错误页）；文案就是核心给的那三类：发生了什么、能做什么、详情。 */
@Preview(name = "错误页", locale = "en", widthDp = 390, heightDp = 420)
@Composable
internal fun ProblemViewPreview() {
    PreviewFrame {
        ProblemView(
            problem = Problem(
                title = "Can't reach your home server",
                nextStep = "Check the network and try again.",
                detail = "error sending request: connection reset by peer",
            ),
            onRetry = {},
        )
    }
}
