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
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
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
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.RunWith

/// 配对 → SAS → 历史解密，以及没有可信设备时的两条恢复路径（ADR-0087、ADR-0076），连真开发栈跑。
/// 账号由外面准备好：旧设备扮演者先发一条加密历史；两条恢复路径跑时旧设备已删，`RECOVERY_KEY` 是当时的恢复密钥。
/// 配对码只能用一次，配对页也只在没配对的 App 上出现：每条用例单跑一次，跑前 `adb shell pm clear app.wonton.android`、
/// 现取一个新码。配对码、历史和密钥都经 instrumentation 参数传入（base64）。
/// 旧设备把表情按序打到 stdout，外面收 `表情：` 那行 `adb push` 到
/// `/sdcard/Android/data/app.wonton.android/files/sas-emoji.txt`（`getExternalFilesDir`），用例对着屏幕比。
/// 跑法：adb shell am instrument -w -e PAIRING_CODE_B64 <码> -e HISTORY_TEXT_B64 <历史> -e RECOVERY_KEY_B64 <密钥>
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
        assertSameEmoji()
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
        // 回列表：密钥还没有，旧消息只能是解不开，从房间里那一行进备份页。
        back()
        waitForText("Hermes", 90_000)
        compose.onNodeWithText("Hermes").performClick()
        waitForText(string(UiR.string.timeline_undecryptable), 60_000)
        saveScreen("history-undecryptable")
        compose.onNodeWithText(string(UiR.string.timeline_key_notice_recovery), substring = true)
            .performClick()
        waitForText(string(UiR.string.chat_backup_title), 30_000)
        // 账号里还留着别的设备时这一页先给“用恢复密钥 / 请另一台设备确认”两个选择。
        val useKey = string(UiR.string.verification_use_recovery_key)
        if (shown(useKey)) compose.onNodeWithText(useKey).performClick()
        val keyField = runCatching {
            compose.waitUntil(30_000) {
                compose.onAllNodesWithTag("chat-backup-recovery-key").fetchSemanticsNodes().isNotEmpty()
            }
        }.isSuccess
        assertTrue("备份页没有恢复密钥输入框", keyField)
        compose.onNodeWithTag("chat-backup-recovery-key").performTextInput(key)
        compose.onNodeWithTag("chat-backup-recover").performScrollTo().performClick()
        // 恢复要先把备份里的密钥从服务器取回来、再解开时间线；负载高时比 120 秒更久。
        waitForText(string(UiR.string.chat_backup_status_enabled), 240_000)
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
        // 密钥在系统验证后面：没设锁屏的模拟器点“查看恢复密钥”就给了；验证卡住时看不到，就只打印一行。
        fun keyShown() = runCatching {
            compose.onAllNodesWithTag("chat-backup-new-key").fetchSemanticsNodes().isNotEmpty()
        }.getOrDefault(false)
        if (!runCatching { compose.waitUntil(10_000) { keyShown() } }.isSuccess) {
            val reveal = string(UiR.string.chat_backup_reveal_key)
            if (shown(reveal)) compose.onNodeWithText(reveal).performScrollTo().performClick()
            runCatching { compose.waitUntil(120_000) { keyShown() } }
        }
        if (keyShown()) {
            val node = compose.onAllNodesWithTag("chat-backup-new-key").fetchSemanticsNodes().first()
            val keyText = node.config[SemanticsProperties.Text].joinToString("") { it.text }
            assertTrue("新密钥为空", keyText.isNotBlank())
            assertNotEquals("新密钥和旧的一样", oldKey, keyText)
            saveScreen("new-key")

            compose.onNodeWithText(string(UiR.string.chat_backup_saved)).performScrollTo().performClick()
            val saved = compose.activity.getString(UiR.string.chat_backup_saved)
            compose.onNode(hasText(saved) and hasAnyAncestor(isDialog())).performClick()
        } else {
            println("新密钥在系统验证后面，没上屏：新密钥比对和保存确认这一步没做")
            saveScreen("no-key")
        }
        saveScreen("save-key")

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

    /// 一条用例一个码、一次 instrument：App 已配对时配对页根本不再出现（`MainActivity` 只在带码的启动 intent 里自动配），
    /// 所以每条前要 `adb shell pm clear app.wonton.android` 并换新码。
    private fun pair() {
        val code = argument("PAIRING_CODE")
        val appeared = runCatching {
            compose.waitUntil(60_000) {
                runCatching { compose.onAllNodesWithTag("pairing-submit").fetchSemanticsNodes().isNotEmpty() }
                    .getOrDefault(false)
            }
        }.isSuccess
        assertTrue(
            "配对页没出现：App 可能已配对。每条用例前先 adb shell pm clear app.wonton.android，再用新配对码",
            appeared,
        )
        compose.onNodeWithTag("pairing-code").performTextInput(code)
        compose.onNodeWithTag("pairing-submit").performScrollTo().performClick()
    }

    /// 交接页的“手边没有别的设备”同时落到备份页。判定可能快过盖层、盖层晚到又压在列表上：
    /// 先等盖层或提醒之一，盖层在就先点它，别把点击打到它底下的列表。
    private fun openBackupBySkipping() {
        val skip = string(UiR.string.handoff_no_other_device)
        val title = string(UiR.string.handoff_title)
        val reminder = string(UiR.string.chat_backup_reminder_enter_key)
        val backup = string(UiR.string.chat_backup_title)
        val deadline = System.currentTimeMillis() + 90_000
        var arrived = false
        while (!arrived && System.currentTimeMillis() < deadline) {
            when {
                shown(backup) -> arrived = true
                shown(skip) -> compose.onNodeWithText(skip).performClick()
                // 交接页起来了、按钮还没画出来：别点到它底下。
                shown(title) -> Thread.sleep(500)
                else -> {
                    if (shown(reminder)) compose.onNodeWithText(reminder).performClick()
                    Thread.sleep(500)
                }
            }
        }
        // 盖层晚到会压住已经打开的备份页：再跳一次，等它收起。
        if (shown(title)) {
            compose.onNodeWithText(skip).performClick()
            waitForGone(title, 30_000)
        }
        waitForText(backup, 30_000)
    }

    private fun shown(text: String): Boolean =
        runCatching { compose.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty() }
            .getOrDefault(false)

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
    /// 两端必须显示同一组表情、同一顺序：对不上就是两台换了不同的东西，主人却点了“相同”。
    private fun assertSameEmoji() {
        val expected = waitForExpectedEmoji()
        assertEquals("旧设备给了几个符号", 7, expected.size)
        val matcher = SemanticsMatcher("本次表情之一") { node ->
            node.config.getOrNull(SemanticsProperties.Text)?.any { it.text in expected } == true
        }
        val shown = compose.onAllNodes(matcher, useUnmergedTree = true).fetchSemanticsNodes()
            .sortedBy { it.boundsInRoot.top }
            .mapNotNull { node ->
                node.config[SemanticsProperties.Text].firstOrNull { it.text in expected }?.text
            }
        assertEquals("两端表情不同：旧设备 $expected，这台 $shown", expected, shown)
    }

    /// 旧设备的 `表情：` 行由外面写进 App 的外部文件目录（`adb push`），符号一个空格隔开。
    private fun waitForExpectedEmoji(): List<String> {
        val file = File(
            InstrumentationRegistry.getInstrumentation().targetContext.getExternalFilesDir(null),
            "sas-emoji.txt",
        )
        val deadline = System.currentTimeMillis() + 120_000
        while (System.currentTimeMillis() < deadline) {
            val raw = if (file.exists()) file.readText().trim() else ""
            if (raw.isNotEmpty()) return raw.split(" ").filter { it.isNotEmpty() }
            Thread.sleep(1_000)
        }
        fail("没收到旧设备的表情：${file.path} 没有内容")
        return emptyList()
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
