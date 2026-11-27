package app.wonton.ui

import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import com.android.resources.Density
import org.junit.Rule
import org.junit.Test

/// 成员页和加入页的截图：竖屏 390×844，和 iOS 的截图夹具对应。
class FamilySnapshotsTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(
            screenWidth = 1170, screenHeight = 2532, xdpi = 440, ydpi = 440,
            density = Density.XHIGH, locale = "en", softButtons = false,
        ),
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
    )

    @Test
    fun snapshotMembersLight() = paparazzi.snapshot { membersLight() }

    @Test
    fun snapshotMembersDark() = paparazzi.snapshot { membersDark() }

    @Test
    fun snapshotJoinPage() = paparazzi.snapshot { joinPage() }
}
