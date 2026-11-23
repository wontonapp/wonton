package app.wonton.android

import android.Manifest
import android.content.ContentValues
import android.graphics.Bitmap
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Base64
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.ui.R as UiR
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/// 配对 → SAS → 历史解密，以及没有可信设备时的两条恢复路径（ADR-0087、ADR-0076），连真开发栈跑。
/// 账号由外面准备好：旧设备扮演者先发一条加密历史；两条恢复路径跑时旧设备已删，`RECOVERY_KEY` 是当时的恢复密钥。
/// 配对码、历史和密钥都经 instrumentation 参数传入。
/// 跑法：adb shell am instrument -e PAIRING_CODE_B64 <base64> -e HISTORY_TEXT_B64 <base64> -e RECOVERY_KEY_B64 <base64>
///   -e class app.wonton.android.SasWalkTest#<用例> app.wonton.android.test/androidx.test.runner.AndroidJUnitRunner
@RunWith(AndroidJUnit4::class)
class SasWalkTest {
    private val compose = createAndroidComposeRule<MainActivity>()

    private val permissions: TestRule =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.CAMERA)
        } else {
            GrantPermissionRule.grant(Manifest.permission.CAMERA)
        }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose)

    @Test
    fun testPairedPhoneGetsTheKeysAndDecryptsHistory() {
        val history = argument("HISTORY_TEXT")
        pair()

        // 新设备配上后整屏是“在另一台设备上继续”，两台对上同一组表情后点“相同”。
        waitForText(string(UiR.string.handoff_title), 90_000)
        saveScreen("handoff-page")
        waitForText("Match", 90_000)
        saveScreen("handoff-compare")
        compose.onNodeWithText("Match").performClick()
        waitForGone(string(UiR.string.handoff_title), 90_000)
        saveScreen("handoff-finished")
        assertHistory(history)
    }

    @Test
    fun testRecoveryKeyRestoresPastMessages() {
        val key = argument("RECOVERY_KEY")
        val history = argument("HISTORY_TEXT")
        pair()
        openBackupBySkipping()

        compose.onNodeWithTag("chat-backup-recovery-key").performTextInput(key)
        if (compose.onAllNodesWithTag("chat-backup-recover").fetchSemanticsNodes().isNotEmpty()) {
            compose.onNodeWithTag("chat-backup-recover").performScrollTo().performClick()
        }
        waitForText(string(UiR.string.chat_backup_status_enabled), 120_000)
        saveScreen("recovered")
        back()
        assertHistory(history)
    }

    @Test
    fun testResetCreatesANewRecoveryKey() {
        val oldKey = argument("RECOVERY_KEY")
        pair()
        openBackupBySkipping()

        compose.onNodeWithTag("chat-backup-reset").performScrollTo().performClick()
        val reset = compose.activity.getString(UiR.string.chat_backup_reset_confirm)
        compose.onNode(hasText(reset) and hasAnyAncestor(isDialog())).performClick()

        waitForText(string(UiR.string.chat_backup_save_key_title), 180_000)
        val newKey = compose.onNodeWithTag("chat-backup-new-key")
        newKey.assertIsDisplayed()
        val keyText = newKey.fetchSemanticsNode().config[SemanticsProperties.Text].joinToString("") { it.text }
        assertTrue("新密钥为空", keyText.isNotBlank())
        assertNotEquals(oldKey, keyText)
        saveScreen("new-key")

        compose.onNodeWithText(string(UiR.string.chat_backup_saved)).performScrollTo().performClick()
        val saved = compose.activity.getString(UiR.string.chat_backup_saved)
        compose.onNode(hasText(saved) and hasAnyAncestor(isDialog())).performClick()

        waitForText(string(UiR.string.chat_backup_status_enabled), 120_000)
        saveScreen("reset-done")
        back()
        // 重置删了旧备份，旧消息不一定还能解密；用新身份发一条，能发出并显示才算真的可继续用。
        waitForText("Hermes", 90_000)
        compose.onNodeWithText("Hermes").performClick()
        val sent = "重置后 ${System.currentTimeMillis()}"
        compose.onNode(hasSetTextAction()).performTextInput(sent)
        compose.onNodeWithContentDescription(string(UiR.string.composer_send)).performClick()
        waitForText(sent, 90_000)
        saveScreen("reset-message")
    }

    private fun string(id: Int): String = compose.activity.getString(id)

    /// 配对码和密钥里有 `&`、`%` 这类字符，经 device shell 传容易被吃掉：参数一律 base64。
    private fun assumeArgument(name: String): String {
        val encoded = InstrumentationRegistry.getArguments().getString("${name}_B64").orEmpty()
        assumeTrue("需要 -e ${name}_B64", encoded.isNotEmpty())
        return String(Base64.decode(encoded, Base64.NO_WRAP))
    }

    private fun argument(name: String) = assumeArgument(name)

    private fun pair() {
        val code = argument("PAIRING_CODE")
        compose.waitUntil(60_000) {
            runCatching { compose.onAllNodesWithTag("pairing-submit").fetchSemanticsNodes().isNotEmpty() }
                .getOrDefault(false)
        }
        compose.onNodeWithTag("pairing-code").performTextInput(code)
        compose.onNodeWithTag("pairing-submit").performScrollTo().performClick()
    }

    /// 交接页的“手边没有别的设备”同时落到备份页。判定可能快到盖层都没出现，这时点列表顶上的提醒。
    private fun openBackupBySkipping() {
        val skip = string(UiR.string.handoff_no_other_device)
        if (runCatching {
                compose.waitUntil(40_000) {
                    compose.onAllNodes(hasText(skip, substring = true)).fetchSemanticsNodes().isNotEmpty()
                }
                true
            }.getOrDefault(false)
        ) {
            compose.onNodeWithText(skip).performClick()
        } else {
            val reminder = string(UiR.string.chat_backup_reminder_enter_key)
            waitForText(reminder, 90_000)
            compose.onNodeWithText(reminder).performClick()
        }
        waitForText(string(UiR.string.chat_backup_title), 30_000)
    }

    private fun waitForText(text: String, timeout: Long) {
        compose.waitUntil(timeout) {
            compose.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun waitForGone(text: String, timeout: Long) {
        compose.waitUntil(timeout) {
            compose.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isEmpty()
        }
    }

    private fun assertHistory(history: String) {
        // 进旧设备发过消息的那个私聊看时间线；列表预览可能已被 Bot 的新回复顶掉。
        waitForText("Hermes", 90_000)
        compose.onNodeWithText("Hermes").performClick()
        waitForText(history, 90_000)
        saveScreen("history-decrypted")
    }

    private fun back() {
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        waitForGone(string(UiR.string.chat_backup_title), 10_000)
    }

    /// 截图进系统相册，跑完由外面 pull。
    private fun saveScreen(name: String) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "sas-walk-$name.png")
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/WontonUITests")
        }
        val uri = requireNotNull(context.contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values))
        requireNotNull(context.contentResolver.openOutputStream(uri)).use { out ->
            compose.onRoot().captureToImage().asAndroidBitmap().compress(Bitmap.CompressFormat.PNG, 100, out)
        }
    }
}
