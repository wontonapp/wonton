package app.wonton.ui

import androidx.compose.runtime.Composable
import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import org.junit.Rule
import org.junit.Test
import java.util.TimeZone

/**
 * 参考图在 `src/androidHostTest/snapshots`，直接进 git。只画预览自己带的固定数据，不连核心、不加载 .so。
 * 录制和比对的命令见 `.github/workflows/README.md`。
 */
// 一张预览一个用例，数量跟着预览面走；拆类没有职责边界，按 main 源集的先例在这里抑制。
@Suppress("TooManyFunctions")
class PreviewSnapshotsTest {
    // 固定一台手机和英文：系统语言换了、设备换了，参考图不该跟着变。软按键不画，画面就是内容本身。
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(locale = "en", softButtons = false),
        // 浅色的系统主题：预览自己铺画布，主题只决定内容之外的底色。
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
    )

    @Test
    fun homeStatus() = paparazzi.snapshot { HomeStatusPanelPreview() }

    @Test
    fun homeStatusDark() = paparazzi.snapshot { HomeStatusPanelDarkPreview() }

    @Test
    fun chatNotices() = paparazzi.snapshot { ChatNoticesPreview() }

    @Test
    fun chatFailure() = paparazzi.snapshot { ChatFailurePreview() }

    @Test
    fun dayBreak() = utcSnapshot { DayBreakPreview() }

    @Test
    fun dayBreakDark() = utcSnapshot { DayBreakDarkPreview() }

    @Test
    fun agentDegraded() = paparazzi.snapshot { AgentDegradedPreview() }

    @Test
    fun agentDegradedInstalledFailure() = paparazzi.snapshot { AgentDegradedInstalledFailurePreview() }

    @Test
    fun agentInstallFailed() = paparazzi.snapshot { AgentInstallFailedPreview() }

    @Test
    fun chatInvite() = paparazzi.snapshot { ChatInvitePreview() }

    @Test
    fun problemView() = paparazzi.snapshot { ProblemViewPreview() }

    @Test
    fun coreUpgradeCompatBreaking() = paparazzi.snapshot { CoreUpgradeCompatBreakingPreview() }

    @Test
    fun coreUpgradeCompatFailed() = paparazzi.snapshot { CoreUpgradeCompatFailedPreview() }

    @Test
    fun connectionDetailsFlows() = utcSnapshot { ConnectionDetailsFlowsPreview() }

    /** 时间按系统时区格式化；预览和 CI（UTC）画出来一致，参考图不随本机时区变。 */
    private fun utcSnapshot(content: @Composable () -> Unit) {
        val previous = TimeZone.getDefault()
        try {
            TimeZone.setDefault(TimeZone.getTimeZone("UTC"))
            paparazzi.snapshot { content() }
        } finally {
            TimeZone.setDefault(previous)
        }
    }

    @Test
    fun shareSheet() = paparazzi.snapshot { ShareSheetPreview() }

    @Test
    fun shareSheetNotPaired() = paparazzi.snapshot { ShareSheetNotPairedPreview() }

    @Test
    fun peersPublicAccessOff() = paparazzi.snapshot { PeersPublicAccessOffPreview() }

    @Test
    fun peersPublicAccessOffDark() = paparazzi.snapshot { PeersPublicAccessOffDarkPreview() }
}
