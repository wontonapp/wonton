package app.wonton.android

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.protocol.PairingFailure
import app.wonton.protocol.PairingStatus
import app.wonton.ui.PairingFailureNotice
import app.wonton.ui.design.WontonTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], application = Application::class)
class PairingFailureNoticeTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun usedCodeOffersRescanWithoutLeavingAndKeepsDetailsCollapsed() {
        var scans = 0
        var exits = 0
        val diagnostic = "diagnostic-token-not-for-the-primary-notice"
        compose.setContent {
            WontonTheme(testAppearance()) {
                PairingFailureNotice(
                    PairingStatus.Failed(PairingFailure.USED, diagnostic),
                    onRetry = {}, onScanAgain = { scans++ }, onBack = { exits++ },
                )
            }
        }
        compose.onNodeWithText(diagnostic).assertDoesNotExist()
        compose.onNodeWithTag("pairing-retry").assertDoesNotExist()
        compose.onNodeWithTag("pairing-scan-again").assertIsDisplayed().performClick()
        compose.runOnIdle {
            assertEquals(1, scans)
            assertEquals(0, exits)
        }
    }

    @Test
    fun transportFailurePreservesSameCodeRetryAndBackActions() {
        var retries = 0
        var exits = 0
        compose.setContent {
            WontonTheme(testAppearance()) {
                PairingFailureNotice(
                    PairingStatus.Failed(PairingFailure.TRANSPORT_UNAVAILABLE, ""),
                    onRetry = { retries++ }, onScanAgain = {}, onBack = { exits++ },
                )
            }
        }
        compose.onNodeWithTag("pairing-retry").performClick()
        compose.onNodeWithTag("pairing-back").performClick()
        compose.runOnIdle {
            assertEquals(1, retries)
            assertEquals(1, exits)
        }
    }
}
