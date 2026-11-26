package app.wonton.ui

import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import org.junit.Rule
import org.junit.Test

/** 添加设备页的收起、展开两态（#666）：配对文本默认收起，展开才显示码；复制按钮两态都在。 */
class InviteSnapshotTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(locale = "en", softButtons = false),
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
    )

    @Test
    fun inviteCollapsed() = paparazzi.snapshot { InviteCodeCollapsedPreview() }

    @Test
    fun inviteExpanded() = paparazzi.snapshot { InviteCodeExpandedPreview() }
}
