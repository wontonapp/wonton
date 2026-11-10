// 预览的假数据：色值、计数和字节数就是字面量，取名字反而看不出画面。
@file:Suppress("MagicNumber")

package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.ChatAccount
import app.wonton.protocol.ChatLayoutChoice
import app.wonton.protocol.ChatSync
import app.wonton.protocol.ChatViewModel
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
import app.wonton.protocol.InstalledPlugins
import app.wonton.protocol.PendingConfirmation
import app.wonton.protocol.PluginsViewModel
import app.wonton.protocol.PushStatus
import app.wonton.protocol.RecoveryStatus
import app.wonton.protocol.RegistryMirrorViewModel
import app.wonton.protocol.Rgb
import app.wonton.protocol.SidebarViewModel
import app.wonton.protocol.StorageUsage
import app.wonton.protocol.TextSize
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
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
    path = HomePath(relay = true, lan = false, rttMs = 48u),
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
        udpFlows = emptyList(),
        udpBufferEvictedDatagrams = 0u,
    ),
    proxy = null,
    webUrl = null,
    pairing = null,
    invite = null,
    keepConnection = false,
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

/**
 * 聊天提示、邀请行和错误页的预览在 ChatPreviews.kt、ProblemPreview.kt 里：
 * ChatScreen.kt 和 ProblemViews.kt 都撞到 detekt 的函数数上限。
 */
