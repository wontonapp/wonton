package app.wonton.ui

import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import org.junit.Rule
import org.junit.Test

/** 引导选项行的四种状态（#672）：未选中、中性选中、装不了、已装但 blocked 被抑制。 */
class StepOptionRowSnapshotTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(locale = "en", softButtons = false),
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
    )

    @Test
    fun stepOptionRowStates() = paparazzi.snapshot { StepOptionRowStatesPreview() }
}
