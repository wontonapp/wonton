package app.wonton.android

import android.app.Application
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.protocol.Action
import app.wonton.protocol.BotContact
import app.wonton.protocol.BotContactsState
import app.wonton.protocol.BotContactsViewModel
import app.wonton.protocol.BotRef
import app.wonton.protocol.ContactError
import app.wonton.protocol.ContactKind
import app.wonton.protocol.ContactState
import app.wonton.ui.BotContactsContent
import app.wonton.ui.R
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.problem
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Bot 联系人页（ADR-0045、ADR-0050）：壳只把点的按钮翻成核心的动作，状态和失败类别由核心给。
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
// 不起 WontonApplication：它的静态初始化要加载 Android 的 arm64 原生库，JVM 上装不进。
@Config(sdk = [36], application = Application::class)
class BotContactsContentTest {
    @get:Rule
    val compose = createComposeRule()

    private val context = ApplicationProvider.getApplicationContext<Application>()
    private val bot = BotRef(pluginId = "hermes", bot = "default")
    private val sent = mutableListOf<Action>()
    private val removed = mutableListOf<BotContact>()

    private fun contact(userId: String, state: ContactState, kind: ContactKind = ContactKind.BOT) = BotContact(
        userId = userId,
        displayName = null,
        keeperDisplayName = null,
        kind = kind,
        state = state,
        remote = null,
    )

    private fun show(view: BotContactsViewModel) {
        compose.setContent {
            WontonTheme(testAppearance()) {
                BotContactsContent(view, bot, { sent += it }, onAdd = {}, onRemove = { removed += it })
            }
        }
    }

    private fun ready(vararg contacts: BotContact) =
        BotContactsViewModel(bot = bot, state = BotContactsState.Ready, contacts = contacts.toList())

    @Test
    fun approvingAndRejectingAnIncomingRequestSendTheMatchingActions() {
        show(ready(contact("@a:b.example", ContactState.INCOMING)))

        compose.onNodeWithText(context.getString(R.string.contacts_approve)).performClick()
        compose.onNodeWithText(context.getString(R.string.contacts_reject)).performClick()

        assertEquals(
            listOf(
                Action.ApproveBotContact(bot, "@a:b.example"),
                Action.RejectBotContact(bot, "@a:b.example"),
            ),
            sent,
        )
    }

    @Test
    fun withdrawingARequestStillWaitingRemovesItWithoutConfirmation() {
        show(ready(contact("@p:b.example", ContactState.PENDING, ContactKind.PERSON)))

        compose.onNodeWithText(context.getString(R.string.contacts_withdraw)).performClick()

        assertEquals(listOf<Action>(Action.RemoveBotContact(bot, "@p:b.example")), sent)
    }

    @Test
    fun removingAnActiveContactAsksTheScreenToConfirmFirst() {
        val active = contact("@c:b.example", ContactState.ACTIVE)
        show(ready(active))

        compose.onNodeWithContentDescription(context.getString(R.string.contacts_remove)).performClick()

        assertEquals(listOf(active), removed)
        assertEquals(emptyList<Action>(), sent)
    }

    @Test
    fun aFailureWithoutRowsOffersARetryThatRefetches() {
        val failed = BotContactsState.Failed(ContactError.TRANSPORT_UNAVAILABLE, "detail")
        show(BotContactsViewModel(bot, failed, emptyList()))

        compose.onNodeWithText(context.getString(R.string.common_retry)).performClick()

        assertEquals(listOf<Action>(Action.RefreshBotContacts(bot)), sent)
    }

    @Test
    fun aCategoryFromANewerCoreFallsBackToTheGenericTitle() {
        val failed = BotContactsState.Failed(ContactError.OTHER, "from a newer core")

        val problem = failed.problem()

        assertEquals(R.string.peers_error_other, problem.titleRes)
        assertEquals("from a newer core", problem.detail)
    }

    @Test
    fun aListForAnotherBotIsNotShownAsThisBots() {
        val other = BotRef(pluginId = "openclaw", bot = "default")
        val incoming = contact("@x:b.example", ContactState.INCOMING)
        show(BotContactsViewModel(other, BotContactsState.Ready, listOf(incoming)))

        compose.onNodeWithText("@x:b.example").assertDoesNotExist()
    }
}
