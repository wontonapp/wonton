package app.wonton.ui

import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import org.junit.Rule
import org.junit.Test

/**
 * 参考图在 `src/androidHostTest/snapshots`，直接进 git。只画预览自己带的固定数据，不连核心、不加载 .so。
 * 录制和比对的命令见 `.github/workflows/README.md`。
 */
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
    fun chatInvite() = paparazzi.snapshot { ChatInvitePreview() }

    @Test
    fun problemView() = paparazzi.snapshot { ProblemViewPreview() }
}
