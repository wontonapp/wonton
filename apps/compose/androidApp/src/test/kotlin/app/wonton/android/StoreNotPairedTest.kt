package app.wonton.android

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.protocol.StoreLoadState
import app.wonton.ui.StorePlaceholder
import app.wonton.ui.design.WontonTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * 商店还没配对时的空态：只给“去配对”，按钮回到上一层（走调用方给的 onPair），
 * 不往核心发动作——没配对时 `HomeAction.AddServer` 会被拒。
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], application = Application::class, qualifiers = "w390dp-h844dp-mdpi")
class StoreNotPairedTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun pairButtonGoesBackWithoutReloading() {
        var pairs = 0
        var reloads = 0
        compose.setContent {
            WontonTheme(testAppearance()) {
                StorePlaceholder(
                    load = StoreLoadState.NotPaired,
                    onReload = { reloads++ },
                    onPair = { pairs++ },
                )
            }
        }
        compose.onNodeWithTag("store-not-paired").assertIsDisplayed()
        compose.onNodeWithTag("store-go-pair").assertIsDisplayed().performClick()
        compose.runOnIdle {
            assertEquals(1, pairs)
            assertEquals(0, reloads)
        }
    }
}
