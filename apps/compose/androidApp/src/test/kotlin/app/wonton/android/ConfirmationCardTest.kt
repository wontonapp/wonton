@file:Suppress("MagicNumber")

package app.wonton.android

import android.app.Application
import android.graphics.Bitmap
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.protocol.Action
import app.wonton.protocol.Answer
import app.wonton.protocol.CardAudience
import app.wonton.protocol.CardChoice
import app.wonton.protocol.ConfirmationCard
import app.wonton.protocol.ConfirmationCategory
import app.wonton.protocol.ConfirmationState
import app.wonton.protocol.ConfirmationTool
import app.wonton.protocol.ConfirmationToolImpact
import app.wonton.protocol.ConfirmationView
import app.wonton.ui.ConfirmationCardView
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], application = Application::class, qualifiers = "w390dp-h844dp-mdpi")
class ConfirmationCardTest {
    @get:Rule
    val compose = createComposeRule()

    private val context = ApplicationProvider.getApplicationContext<Application>()
    private val rawName = "backup__remove_old"
    private val arguments = "{\n  \"archive_ids\": [\"backup-123\", \"backup-456\"]\n}"
    private val details get() = context.getString(R.string.common_details)

    private fun fixture(id: String = "first", target: String? = null) = ConfirmationView(
        card = ConfirmationCard(
            id = id,
            state = ConfirmationState.Pending,
            title = "Remove old backups",
            lines = emptyList(),
            choices = listOf(CardChoice(Answer.APPROVE, "Continue", "approve-once")),
            expiresAtMs = 0u,
            window = null,
            pluginId = null,
            secretInputs = emptyList(),
            settings = null,
            requester = null,
            category = ConfirmationCategory.ToolCall(
                ConfirmationTool(rawName, "Backups", arguments, ConfirmationToolImpact.Destructive(target), false),
            ),
            origin = null,
            audience = CardAudience.KEEPER,
            answeredBy = null,
            permissions = emptyList(),
            devices = emptyList(),
        ),
        sending = null,
        sendingChoiceId = null,
        problem = null,
        window = null,
    )

    @Test
    fun detailsStartCollapsedToggleAndResetForAnotherCard() {
        var view by mutableStateOf(fixture())
        compose.setContent {
            WontonTheme(testAppearance()) {
                Box(Modifier.requiredSize(390.dp, 844.dp)) { ConfirmationCardView(view) {} }
            }
        }
        compose.onNodeWithText(arguments, substring = true).assertDoesNotExist()
        compose.onNodeWithText(rawName, substring = true).assertDoesNotExist()
        capture("confirmation-collapsed")
        compose.onNodeWithText(details).performClick()
        compose.onNodeWithText("$rawName\n\n$arguments").assertIsDisplayed()
        capture("confirmation-expanded")
        compose.onNodeWithText(details).performClick()
        compose.onNodeWithText(arguments, substring = true).assertDoesNotExist()
        compose.onNodeWithText(details).performClick()
        compose.runOnIdle { view = fixture(id = "second") }
        compose.onNodeWithText(arguments, substring = true).assertDoesNotExist()
    }

    @Test
    fun destructiveTargetsStayVisibleAndChoicePreservesItsAction() {
        var view by mutableStateOf(fixture())
        val actions = mutableListOf<Action>()
        compose.setContent {
            WontonTheme(testAppearance()) { ConfirmationCardView(view) { actions += it } }
        }
        compose.onNodeWithText(context.getString(R.string.confirmation_card_tool_destructive_unknown))
            .assertIsDisplayed()
        compose.runOnIdle { view = fixture(target = "November backups") }
        compose.onNodeWithText(
            context.getString(R.string.confirmation_card_tool_destructive_target, "November backups"),
        ).assertIsDisplayed()
        compose.onNodeWithText(arguments, substring = true).assertDoesNotExist()
        compose.onNodeWithText("Continue").performClick()
        assertEquals(
            listOf(
                Action.AnswerConfirmation(
                    "first", Answer.APPROVE, "approve-once", emptyMap(), emptyMap(), emptyList(),
                ),
            ),
            actions,
        )
    }

    private fun capture(name: String) {
        val directory = System.getenv("WONTON_CONFIRMATION_SCREENSHOTS") ?: return
        val folder = File(directory)
        assertTrue("Screenshot directory must be absolute", folder.isAbsolute)
        assertTrue("Screenshot directory could not be created", folder.isDirectory || folder.mkdirs())
        val image = compose.onRoot().captureToImage().asAndroidBitmap()
        File(folder, "$name.png").outputStream().use { output ->
            assertTrue(image.compress(Bitmap.CompressFormat.PNG, 100, output))
        }
    }
}
