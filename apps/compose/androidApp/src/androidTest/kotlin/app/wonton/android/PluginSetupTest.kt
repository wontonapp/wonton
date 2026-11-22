package app.wonton.android

import android.graphics.Bitmap
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.longClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.R
import app.wonton.ui.send
import java.io.File
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** UI-FAKE LAYOUT ONLY：详情页在描述下面写首次打开要做的一步，不用点开安装。 */
@RunWith(AndroidJUnit4::class)
class PluginSetupTest {
    @get:Rule
    val compose = createAndroidComposeRule<UiFakeMainActivity>()

    @Test
    fun setupShowsUnderTheDescription() {
        openHomeHubDetail()
        val heading = compose.activity.getString(R.string.plugin_setup_title)
        compose.onNodeWithText(heading).performScrollTo().assertIsDisplayed()
        compose.onNodeWithText(STEP).performScrollTo().assertIsDisplayed()
        compose.waitForIdle()
        val bitmap = requireNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot())
        try {
            val directory = File(compose.activity.getExternalFilesDir(null), "plugin-setup")
            assertTrue(directory.isDirectory || directory.mkdirs())
            File(directory, "UI-FAKE-LAYOUT-ONLY-setup.png").outputStream().use {
                assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it))
            }
        } finally {
            bitmap.recycle()
        }
    }

    private fun openHomeHubDetail() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasTestTag("sidebar-open")).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("sidebar-open").performClick()
        compose.onNodeWithTag("sidebar-all-apps").performClick()
        compose.onNodeWithText("Home Hub").performScrollTo().performTouchInput { longClick() }
        compose.onNodeWithText(compose.activity.getString(R.string.common_details)).performClick()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasTestTag("plugin-granted-devices")).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private companion object {
        const val WAIT_MS = 20_000L
        const val STEP = "Sign in as admin, then change the password."
    }
}
