@file:Suppress("MagicNumber") // 注入的系统栏值（24/16dp）本来就是测试自定的。

package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import app.wonton.client.TabEnvironment
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

/**
 * 整页的安全区分摊：`safe` 留给胶囊和浮层，页面环境顶部报 0（Android 状态栏区域吃点击，网页由壳垫下移；
 * 照实报的话按桥适配过安全区的页面会双重让位）、底部照实（Home 条还在网页下面）。
 * 参考图上半条是状态栏让空区（画布色），下面是网页从状态栏下沿开始；系统栏的值由测试注入，
 * 不依赖运行环境给的真实 insets（JVM 布局渲染里恒为 0）。
 */
class PageInsetsTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(locale = "en", softButtons = false),
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
    )

    @Test
    fun pageEnvironmentSplitsInsets() {
        var environment: TabEnvironment? = null
        var overlayTop = 0.dp
        paparazzi.snapshot {
            val viewport = tabViewport(true, PaddingValues(top = 24.dp, bottom = 16.dp))
            environment = viewport.environment
            overlayTop = viewport.safe.calculateTopPadding()
            Column(Modifier.fillMaxSize()) {
                Box(Modifier.fillMaxWidth().height(24.dp))
                Box(Modifier.fillMaxSize().background(Color(0xFFDADADA)))
            }
        }
        val env = environment ?: error("viewport not composed")
        assertEquals(0.0, env.insetTop, 0.0)
        assertEquals(0.0, env.safeArea.top, 0.0)
        assertEquals(16.0, env.insetBottom, 0.0)
        assertEquals(16.0, env.safeArea.bottom, 0.0)
        assertEquals(24.dp, overlayTop)
    }
}
