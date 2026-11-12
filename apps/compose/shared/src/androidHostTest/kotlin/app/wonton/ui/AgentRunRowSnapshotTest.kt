package app.wonton.ui

import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import org.junit.Rule
import org.junit.Test

/** Agent 一轮运行状态行的全部状态（ADR-0082）：一张参考图对着 [AgentRunRowStatesPreview]。 */
class AgentRunRowSnapshotTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(locale = "en", softButtons = false),
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
    )

    @Test
    fun agentRunStates() = paparazzi.snapshot { AgentRunRowStatesPreview() }
}
