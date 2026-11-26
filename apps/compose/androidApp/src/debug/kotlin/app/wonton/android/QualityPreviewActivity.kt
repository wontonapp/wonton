@file:Suppress("MagicNumber") // 走查用的假数据：假外观的色值和无意义的计数就是字面量。

package app.wonton.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.CardLine
import app.wonton.protocol.ConnectionCheck
import app.wonton.protocol.ConnectionDetails
import app.wonton.protocol.ConnectionQuality
import app.wonton.protocol.DeviceNetwork
import app.wonton.protocol.FontStyle
import app.wonton.protocol.HomePath
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.OnboardingCommit
import app.wonton.protocol.OnboardingItemState
import app.wonton.protocol.OnboardingReview
import app.wonton.protocol.OnboardingReviewDetail
import app.wonton.protocol.OnboardingReviewItem
import app.wonton.protocol.OnboardingReviewKind
import app.wonton.protocol.PermissionLine
import app.wonton.protocol.PermissionRisk
import app.wonton.protocol.RelayCurrent
import app.wonton.protocol.RelaySettingEntry
import app.wonton.protocol.RelaySettingsPhase
import app.wonton.protocol.RelaySettingsView
import app.wonton.protocol.RelaySource
import app.wonton.protocol.RelayState
import app.wonton.protocol.Rgb
import app.wonton.protocol.StepNav
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.ThemeMode
import app.wonton.protocol.TextSize
import app.wonton.protocol.UdpFlowStats
import app.wonton.ui.ConnectionDetailsPage
import app.wonton.ui.OnboardingReviewStep
import app.wonton.ui.PluginsState
import app.wonton.ui.R
import app.wonton.ui.connectionBarStatus
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.StepCardFrame
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRootBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme

/**
 * #847 的界面走查和引导确认页：用假数据把状态摆出来。`-e screen poor|relay|details|details-flows|onboarding-review` 选页。
 * 只在 debug 构建注册（src/debug/AndroidManifest.xml），不进 release。
 */
class QualityPreviewActivity : ComponentActivity() {
    private var screen by mutableStateOf(SCREEN_POOR)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        screen = intent?.getStringExtra(EXTRA_SCREEN) ?: SCREEN_POOR
        setContent {
            WontonTheme(previewAppearance()) {
                when (screen) {
                    SCREEN_RELAY -> RelayBarPreview()
                    SCREEN_DETAILS -> ConnectionDetailsPage(previewHome(relay = true), null, onBack = {})
                    SCREEN_ONBOARDING_REVIEW -> OnboardingReviewPreview()
                    SCREEN_DETAILS_FLOWS -> ConnectionDetailsPage(
                        previewHome(relay = true, flows = previewUdpFlows(), evicted = 57uL),
                        null,
                        onBack = {},
                    )
                    else -> PoorBarPreview()
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        screen = intent.getStringExtra(EXTRA_SCREEN) ?: SCREEN_POOR
    }

    /** 根页面：短状态替换页面标题。 */
    @Composable
    private fun PoorBarPreview() {
        WithBarStatus(previewHome(relay = false)) {
            WontonScreen(bar = { WontonRootBar(stringResource(R.string.launcher_title)) }) {}
        }
    }

    /** 引导最后一步：三类汇总、权限、来源提示和安装进展。 */
    @Composable
    private fun OnboardingReviewPreview() {
        val core = (application as WontonApplication).core.getOrNull() ?: return
        val plugins = remember(core) { PluginsState(core) }
        val scroll = rememberScrollState()
        Box(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
        StepCardFrame(
            nav = StepNav(position = 4u, total = 4u, canBack = true, canSkip = false, canNext = true, canClose = true),
            title = stringResource(R.string.onboarding_step_review),
            primaryText = stringResource(R.string.onboarding_done),
            primaryBusy = false,
            onBack = {},
            onSkip = {},
            onPrimary = {},
        ) {
            Column(Modifier.weight(1f).verticalScroll(scroll)) {
                OnboardingReviewStep(previewOnboardingReview(), plugins) {}
            }
        }
        }

    }

    /** 聊天房间：短状态替换副标题。 */
    @Composable
    private fun RelayBarPreview() {
        WithBarStatus(previewHome(relay = true)) {
            WontonScreen(bar = {
                WontonPushedBar(stringResource(R.string.tab_chat), onBack = {}, showsStatus = true)
            }) {}
        }
    }

    @Composable
    private fun WithBarStatus(home: HomeViewModel, content: @Composable () -> Unit) {
        CompositionLocalProvider(
            LocalWontonBarStatus provides connectionBarStatus(home, {}, {}),
        ) { content() }
    }
}

private const val EXTRA_SCREEN = "screen"
private const val SCREEN_POOR = "poor"
private const val SCREEN_RELAY = "relay"
private const val SCREEN_DETAILS = "details"
private const val SCREEN_DETAILS_FLOWS = "details-flows"
private const val SCREEN_ONBOARDING_REVIEW = "onboarding-review"

/** 确认页的假数据：资料和模型没改，一台要装的应用带两项权限和来源提示，一台在排队。 */
private fun previewOnboardingReview() = OnboardingReview(
    items = listOf(
        OnboardingReviewItem(
            id = "profile",
            kind = OnboardingReviewKind.PROFILE,
            titles = listOf("Ada Lovelace"),
            detail = OnboardingReviewDetail.Unchanged,
            icon = null,
            permissions = emptyList(),
            notices = emptyList(),
            state = OnboardingItemState.Ready,
        ),
        OnboardingReviewItem(
            id = "model",
            kind = OnboardingReviewKind.MODEL,
            titles = listOf("OpenAI"),
            detail = OnboardingReviewDetail.Unchanged,
            icon = null,
            permissions = emptyList(),
            notices = emptyList(),
            state = OnboardingItemState.Ready,
        ),
        OnboardingReviewItem(
            id = "app.immich.photos",
            kind = OnboardingReviewKind.APP,
            titles = listOf("家庭相册"),
            detail = null,
            icon = null,
            permissions = listOf(
                PermissionLine(
                    permission = "storage.share.photos:read",
                    text = "读共享文件夹 photos",
                    risk = null,
                    notice = null,
                    supported = true,
                    added = false,
                    optional = false,
                    enabled = false,
                    offLoss = null,
                    opened = null,
                ),
                PermissionLine(
                    permission = "devices.gpu",
                    text = "用显卡（高权限）",
                    risk = PermissionRisk.HIGH,
                    notice = null,
                    supported = true,
                    added = false,
                    optional = false,
                    enabled = false,
                    offLoss = null,
                    opened = null,
                ),
            ),
            notices = listOf(
                CardLine(text = "云吞没审核过这个应用，程序由 Example 提供", emphasis = true),
                CardLine(text = "程序没有锁定具体版本：example/app:latest", emphasis = false),
            ),
            state = OnboardingItemState.Ready,
        ),
        OnboardingReviewItem(
            id = "app.memos",
            kind = OnboardingReviewKind.APP,
            titles = listOf("随手记"),
            detail = null,
            icon = null,
            permissions = emptyList(),
            notices = emptyList(),
            state = OnboardingItemState.Waiting,
        ),
    ),
    commit = OnboardingCommit.NOT_STARTED,
)

private fun previewHome(
    relay: Boolean,
    flows: List<UdpFlowStats> = emptyList(),
    evicted: ULong? = null,
) = HomeViewModel(
    deviceId = "0".repeat(64),
    serverId = null,
    status = HomeStatus.Connected,
    path = HomePath(
        relay = relay,
        publicRelay = false,
        lan = false,
        rttMs = 1240u,
        relayUrl = if (relay) "https://relay.example.org/" else null,
    ),
    currentRelay = if (relay) {
        RelayCurrent(url = "https://relay.example.org/", rttMs = 1240u)
    } else {
        null
    },
    relays = previewRelays(),
    notice = null,
    details = ConnectionDetails(
        network = DeviceNetwork.WIFI,
        relay = ConnectionCheck.OK,
        server = ConnectionCheck.OK,
        lastConnectedAtMs = System.currentTimeMillis().toULong(),
        handshakeMs = 113u,
        disconnects = 2u,
        failedDials = 2u,
        sentPackets = 3141uL,
        lostPackets = 232uL,
        quality = ConnectionQuality.POOR,
        lossPerMille = 73u.toUShort(),
        rttMs = 1240u,
        lastFailure = null,
        diagnostic = null,
        addressesStale = false,
        udpFlows = flows,
        udpBufferEvictedDatagrams = evicted,
    ),
    proxy = null,
    webUrl = null,
    pairing = null,
    invite = null,
    keepConnection = false,
    justRecovered = false,
)

private fun previewRelays() = RelaySettingsView(
    phase = RelaySettingsPhase.Idle,
    entries = listOf(
        RelaySettingEntry(
            url = "https://relay.example.org/",
            token = null,
            quicPort = null,
            tier = 0u.toUByte(),
            source = RelaySource.BUILTIN,
            state = RelayState.Connected,
            editable = true,
        ),
        RelaySettingEntry(
            url = "https://relay.example.net/",
            token = "wonton-relay-token",
            quicPort = 7842u.toUShort(),
            tier = 1u.toUByte(),
            source = RelaySource.SETTINGS,
            state = RelayState.Unknown,
            editable = true,
        ),
    ),
)

private fun previewUdpFlows() = listOf(
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
)

// 和 StepCardFrameTest 的假外观同一套颜色。
private fun rgb(hex: Int) = Rgb(
    (hex shr 16 and 0xFF).toUByte(),
    (hex shr 8 and 0xFF).toUByte(),
    (hex and 0xFF).toUByte(),
)

private fun palette() = ThemeColors(
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

private fun previewAppearance() = AppearanceViewModel(
    accent = rgb(0x3366FF),
    presets = listOf(rgb(0x3366FF)),
    moreColors = emptyList(),
    swatchColumns = 6u,
    themeMode = ThemeMode.SYSTEM,
    themeModes = listOf(ThemeMode.SYSTEM),
    textSize = TextSize.STANDARD,
    textSizes = listOf(TextSize.STANDARD),
    font = FontStyle.DEFAULT,
    fonts = listOf(FontStyle.DEFAULT),
    light = palette(),
    dark = palette(),
    problem = null,
)
