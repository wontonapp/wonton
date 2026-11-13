package app.wonton.ui

import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import com.android.resources.Density
import org.junit.Rule
import org.junit.Test

class AdaptiveChatSnapshotsTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(
            screenWidth = 2048, screenHeight = 1536, xdpi = 320, ydpi = 320,
            density = Density.XHIGH,
            locale = "en", softButtons = false,
        ),
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
    )

    @Test
    fun wideChat() = paparazzi.snapshot { WideChatPreview() }
}
