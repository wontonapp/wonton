@file:Suppress("MagicNumber") // Device pixel sampling geometry and screenshot tolerance.

package app.wonton.android

import android.Manifest
import android.content.ContentValues
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.view.KeyEvent
import android.view.WindowInsets
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.assertCountEquals
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.Rgb
import app.wonton.protocol.ThemeMode
import app.wonton.protocol.Action
import app.wonton.protocol.SettingsAction
import app.wonton.ui.send
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith
import kotlin.math.abs

/** Run on a fresh, unpaired portrait emulator with visible status and gesture navigation bars. */
@RunWith(AndroidJUnit4::class)
class SafeAreaBackgroundTest {
    private val compose = createAndroidComposeRule<MainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.CAMERA)
    } else {
        GrantPermissionRule.grant(Manifest.permission.CAMERA)
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun lightNativePagesPaintBehindSystemBars() = checkPages(ThemeMode.LIGHT)

    @Test
    fun darkNativePagesPaintBehindSystemBars() = checkPages(ThemeMode.DARK)

    private fun checkPages(mode: ThemeMode) {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertTrue("Requires a fresh unpaired app", core.home().status is HomeStatus.NotConfigured)
        val original = core.appearance().themeMode
        try {
            assertNull(core.send(Action.Settings(SettingsAction.SetThemeMode(mode))))
            awaitRoot()
            val appearance = core.appearance()
            val colors = if (mode == ThemeMode.DARK) appearance.dark else appearance.light
            val prefix = mode.name.lowercase()
            checkPixels("$prefix-chat", colors.canvas)
            checkControlInsets("sidebar-open")

            compose.onNodeWithTag("sidebar-open").performClick()
            compose.waitForIdle()
            checkControlInsets("sidebar-settings")
            checkControlInsets("sidebar-all-apps")
            checkPixels("$prefix-sidebar", colors.surfaceSubtle)
            compose.onNodeWithTag("sidebar-close").performClick()
            compose.waitForIdle()
            checkPixels("$prefix-sidebar-closed", colors.canvas)

            compose.onNodeWithTag("sidebar-open").performClick()
            compose.onNodeWithTag("sidebar-all-apps").performClick()
            compose.waitForIdle()
            // The real unpaired My Apps route displays pairing, not a fabricated launcher fixture.
            compose.onNodeWithTag("pairing-submit").assertExists()
            // The legacy native-root action can show a rejection over the unpaired route.
            // Check the page canvas, not the separate dialog window's intentional dim layer.
            if (compose.onAllNodes(isDialog()).fetchSemanticsNodes().isNotEmpty()) {
                instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
                compose.waitForIdle()
            }
            compose.onAllNodes(isDialog()).assertCountEquals(0)
            checkPixels("$prefix-my-apps", colors.canvas)
            compose.onNodeWithTag("sidebar-open").performClick()
            compose.onNodeWithTag("sidebar-settings").performClick()
            compose.waitForIdle()
            checkPixels("$prefix-settings", colors.canvas)
        } finally {
            assertNull(core.send(Action.Settings(SettingsAction.SetThemeMode(original))))
        }
    }

    private fun awaitRoot() {
        compose.waitUntil(timeoutMillis = 60_000) {
            runCatching { compose.onAllNodesWithTag("sidebar-open").fetchSemanticsNodes().isNotEmpty() }
                .getOrDefault(false)
        }
        compose.waitForIdle()
    }

    private fun systemInsets(): android.graphics.Insets = compose.runOnIdle {
        val root = compose.activity.window.decorView.rootWindowInsets
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            root.getInsets(WindowInsets.Type.systemBars())
        } else {
            @Suppress("DEPRECATION")
            android.graphics.Insets.of(
                root.systemWindowInsetLeft, root.systemWindowInsetTop,
                root.systemWindowInsetRight, root.systemWindowInsetBottom,
            )
        }
    }

    private fun checkControlInsets(tag: String) {
        val insets = systemInsets()
        val bounds = compose.onNodeWithTag(tag).fetchSemanticsNode().boundsInWindow
        val height = compose.runOnIdle { compose.activity.window.decorView.height }
        assertTrue("$tag overlaps status bar", bounds.top >= insets.top)
        assertTrue("$tag overlaps navigation bar", bounds.bottom <= height - insets.bottom)
    }

    private fun checkPixels(name: String, expected: Rgb) {
        compose.waitForIdle()
        instrumentation.waitForIdleSync()
        // Compose can be idle while Android is still fading the launch window over its first frame.
        instrumentation.uiAutomation.waitForIdle(500, 5_000)
        val insets = systemInsets()
        assertTrue("Status bar must be visible", insets.top > 0)
        assertTrue("Navigation bar must be visible", insets.bottom > 0)
        // captureToImage() excludes the system window: UiAutomation includes both system bars.
        val bitmap = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        try {
            saveScreenshot(name, bitmap)
            val decor = compose.runOnIdle { compose.activity.window.decorView }
            assertEquals("Must capture the full window width", decor.width, bitmap.width)
            assertEquals("Must capture the full window height", decor.height, bitmap.height)
            // Left margin is clear of cards, sidebar rows and the shifted main-page shadow.
            val gutter = (bitmap.width * 0.01f).toInt().coerceAtLeast(1)
            assertPatch(bitmap, expected, gutter, gutter + 2, bitmap.height / 2, bitmap.height / 2 + 3, name)
            // Avoid clock/icons/cutout and the centered navigation handle; cover a band, not one lucky pixel.
            val left = bitmap.width * 22 / 100
            val right = bitmap.width * 30 / 100
            assertPatch(bitmap, expected, left, right, insets.top / 4, insets.top * 3 / 4, "$name status")
            assertPatch(
                bitmap, expected, left, right,
                bitmap.height - insets.bottom * 3 / 4, bitmap.height - insets.bottom / 4, "$name navigation",
            )
        } finally {
            bitmap.recycle()
        }
    }

    private fun assertPatch(
        bitmap: Bitmap, expected: Rgb, left: Int, right: Int, top: Int, bottom: Int, label: String,
    ) {
        assertTrue("$label sample must not be empty", right > left && bottom > top)
        val cutouts = compose.runOnIdle {
            compose.activity.window.decorView.rootWindowInsets.displayCutout?.boundingRects.orEmpty()
        }
        var sampled = 0
        for (y in top until bottom) {
            for (x in left until right) {
                if (cutouts.any { it.contains(x, y) }) continue
                sampled++
                val pixel = bitmap.getPixel(x, y)
                val difference = maxOf(
                    abs(Color.red(pixel) - expected.red.toInt()),
                    abs(Color.green(pixel) - expected.green.toInt()),
                    abs(Color.blue(pixel) - expected.blue.toInt()),
                )
                assertTrue("$label ($x,$y): ${Integer.toHexString(pixel)} differs from $expected", difference <= 3)
            }
        }
        assertTrue("$label must sample pixels outside the display cutout", sampled > 0)
    }

    private fun saveScreenshot(name: String, bitmap: Bitmap) {
        val resolver = instrumentation.targetContext.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "safe-area-$name.png")
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/WontonUITests")
        }
        val uri = requireNotNull(resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values))
        requireNotNull(resolver.openOutputStream(uri)).use { output ->
            assertTrue("Screenshot encoding failed", bitmap.compress(Bitmap.CompressFormat.PNG, 100, output))
        }
    }
}
