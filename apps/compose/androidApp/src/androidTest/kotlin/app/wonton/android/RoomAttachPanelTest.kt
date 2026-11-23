package app.wonton.android

import android.content.ContentValues
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.R
import kotlinx.coroutines.runBlocking
import org.junit.Test

/** 附件面板和转发面板（ADR-0109 第 9 节）：附言就是草稿、选完就发、被拒留住字和附件。 */
class RoomAttachPanelTest : RoomTelegramCase() {
    @Test
    fun attachSheetSendsPhotosWithDraftAsCaption() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        val picks = ensureAlbumPhotos()
        clearTray()
        // 输入框里先打一句：选了照片、附言条出来时里面就是它，面板改的也是同一份草稿。
        compose.onNode(hasSetTextAction()).performTextInput("先说一句")
        openAlbum()
        photos()[albumIndexOf(picks[0])].performClick()
        photos()[albumIndexOf(picks[1])].performClick()
        compose.waitUntil(10_000) { photos()[albumIndexOf(picks[0])].isSelected() }
        // 附言条只在选了照片以后才出来（ADR-0109 第 9 节），选了第一张再找它。
        val field = compose.onNode(hasSetTextAction() and hasAnyAncestor(hasTestTag("attach-send")))
        field.assert(hasText("先说一句"))
        field.performTextClearance()
        field.performTextInput("两张都发")
        compose.waitForIdle()
        val composerField = compose.onNode(hasSetTextAction() and !hasAnyAncestor(hasTestTag("attach-send")))
        composerField.assert(hasText("两张都发"))
        screenshot("attach-sheet")
        sheetSend("attach-send").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty()
        }
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasSetTextAction() and hasText("")).fetchSemanticsNodes().isNotEmpty()
        }
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasText("两张都发", substring = true)).fetchSemanticsNodes().isNotEmpty()
        }
        val vm = runBlocking { core.subscribeComposer(TimelineTarget(ECHO_ROOM, null)).use { it.next()!! } }
        check(vm.attachments.isEmpty()) { "附件区还剩 ${vm.attachments.size} 条" }
        screenshot("attach-sent")
        check(trayTiles() == 0) { "附件区里还剩着照片" }
        // 发出去了才清草稿：输入框现在是空的。
        compose.onNode(hasSetTextAction()).assertIsDisplayed().assert(hasText(""))
    }

    /** 附件区里有一张没传上去的照片时点发送：核心拒收，字、附件和回复对象都得留着，原因说出来。 */
    @Test
    fun rejectedAttachKeepsCaptionInTheInput() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        val picks = ensureAlbumPhotos()
        clearTray()
        // 面板选完就发，先在核心那边定好：下一张进附件区的照片上传失败。
        core.failNextAttachmentForTest(TimelineTarget(ECHO_ROOM, null))
        openAlbum()
        photos()[albumIndexOf(picks[0])].performClick()
        compose.waitUntil(10_000) { photos()[albumIndexOf(picks[0])].isSelected() }
        sheetSend("attach-send").performClick()
        compose.waitUntil(10_000) { trayTiles() == 1 }
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty() }
        // 左滑一条当回复对象：被拒以后它和字一起留着。
        scrollTo(15)
        item(15).performTouchInput {
            val middle = height / 2f
            swipe(Offset(width * 0.8f, middle), Offset(width * 0.3f, middle), durationMillis = 300)
        }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
        // 附言就是草稿：打一句再发，被拒以后这句还在输入框里。
        compose.onNode(hasSetTextAction()).performTextInput("先发这句")
        compose.onNodeWithContentDescription(string(R.string.composer_send)).performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasText("Retry or remove it first", substring = true)).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
        compose.onNode(hasText("先发这句")).assertIsDisplayed()
        check(compose.onAllNodes(hasText("先发这句")).fetchSemanticsNodes().size == 1) {
            "被拒的这句也进了时间线"
        }
        check(trayTiles() == 1) { "附件被吃掉了" }
        // 这张一直标着上传失败，留着会让后面用例的发送也被核心拒掉。
        clearTray()
    }

    /** 转码期间又打的字留着：发出去的是按下时的那句附言，等的时候补的字回到输入框。 */
    @Test
    fun textTypedWhileSendingStaysInTheInput() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        val picks = ensureAlbumPhotos()
        clearTray()
        core.holdNextAttachmentForTest(TimelineTarget(ECHO_ROOM, null))
        openAlbum()
        photos()[albumIndexOf(picks[0])].performClick()
        compose.waitUntil(10_000) { photos()[albumIndexOf(picks[0])].isSelected() }
        val field = compose.onNode(hasSetTextAction() and hasAnyAncestor(hasTestTag("attach-send")))
        field.performTextInput("先说一句")
        sheetSend("attach-send").performClick()
        // 照片进了附件区、停在这里：附言框里再补一句。
        compose.waitUntil(10_000) { trayTiles() == 1 }
        field.performTextInput("等的时候补的")
        compose.waitForIdle()
        core.releaseHeldAttachmentForTest(TimelineTarget(ECHO_ROOM, null))
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty() }
        // 发出去的是按下时那句；补的那句还在输入框里，没跟着走也没被清掉。
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasText("先说一句")).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNode(hasSetTextAction()).assert(hasText("等的时候补的"))
        check(trayTiles() == 0) { "附件区里还剩着照片" }
    }

    /** 转码期间再点一下发送不会发两遍：键灰着，第二下不起作用。 */
    @Test
    fun secondTapWhileSendingDoesNotSendTwice() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        val picks = ensureAlbumPhotos()
        clearTray()
        core.holdNextAttachmentForTest(TimelineTarget(ECHO_ROOM, null))
        openAlbum()
        photos()[albumIndexOf(picks[0])].performClick()
        compose.waitUntil(10_000) { photos()[albumIndexOf(picks[0])].isSelected() }
        val sentBefore = photoMessages()
        sheetSend("attach-send").performClick()
        compose.waitUntil(10_000) { trayTiles() == 1 }
        sheetSend("attach-send").assertIsNotEnabled()
        sheetSend("attach-send").performClick()
        core.releaseHeldAttachmentForTest(TimelineTarget(ECHO_ROOM, null))
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty() }
        check(trayTiles() == 0) { "附件区里还剩着照片" }
        check(photoMessages() == sentBefore + 1) { "发了 ${photoMessages() - sentBefore} 条照片" }
    }

    /**
     * 转发面板：搜索能收窄列表，勾上两个房间后一起发出去，谁也没跳走，两个房间都有一条转发来的。
     * 附言按定案拿出去，等核心的 `ForwardMessage.comment`。
     */
    @Test
    fun forwardPickerSendsToEveryTarget() {
        openForward()
        compose.waitUntil(10_000) { forwardRows().fetchSemanticsNodes().size == 2 }
        sheetSend("forward-send").assertIsNotEnabled()
        compose.onNodeWithTag("forward-search").performTextInput("Fam")
        compose.waitUntil(10_000) { forwardRows().fetchSemanticsNodes().size == 1 }
        compose.onNode(hasTestTag("forward-row") and hasText("Family")).performClick()
        compose.onNodeWithTag("forward-search").performTextClearance()
        compose.waitUntil(10_000) { forwardRows().fetchSemanticsNodes().size == 2 }
        compose.onNode(hasTestTag("forward-row") and hasText("Notes")).performClick()
        sheetSend("forward-send").assertIsEnabled()
        screenshot("forward-sheet")
        sheetSend("forward-send").performClick()
        // 勾了两个：留在原房间，谁也没跳走。
        compose.waitForIdle()
        check(
            compose.onAllNodes(hasText(ECHO_NAME) and hasAnyAncestor(hasTestTag("room-bar")))
                .fetchSemanticsNodes().isNotEmpty(),
        ) { "原房间跳走了" }
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.waitForIdle()
        if (hasBack()) compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        // 到了聊天列表，依次进 Family 和 Notes，两边都得有转发来的那条。
        compose.onNodeWithText("Family").performClick()
        compose.waitUntil(10_000) { forwardedInto("Family", "ui-fake-forwarded-24") }
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.waitForIdle()
        if (hasBack()) compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.onNodeWithText("Notes").performClick()
        compose.waitUntil(10_000) { forwardedInto("Notes", "ui-fake-forwarded-24") }
    }

    /** 只勾一个时发完跟过去：那个房间打开，转发也在。 */
    @Test
    fun forwardToSingleTargetOpensIt() {
        openForward()
        compose.waitUntil(10_000) { forwardRows().fetchSemanticsNodes().size == 2 }
        compose.onNode(hasTestTag("forward-row") and hasText("Family")).performClick()
        sheetSend("forward-send").performClick()
        compose.waitUntil(10_000) { forwardedInto("Family", "ui-fake-forwarded-24") }
    }

    /** 旁聊里一摞能选好几张（和 Telegram 的十张一样），发送键跟着选中数启用。 */
    @Test
    fun sideChatAlbumTakesSeveralPhotos() {
        enterSideChat()
        val picks = ensureAlbumPhotos()
        val first = albumIndexOf(picks[0])
        val second = albumIndexOf(picks[1])
        openAlbum()
        compose.waitUntil(10_000) { photos().fetchSemanticsNodes().size >= 2 }
        photos()[first].performClick()
        compose.waitForIdle()
        // 旁聊没有附件区，但一摞能选好几张：选了一张，后面的照样点得动。
        photos()[second].assertIsEnabled().performClick()
        compose.waitForIdle()
        photos()[first].assertIsSelected()
        photos()[second].assertIsSelected()
        sheetSend("attach-send").assertIsEnabled()
    }

    /** 旁聊里一摞带回复：只有第一张带回复对象，发出去以后回复条就收了。 */
    @Test
    fun sideChatPhotosOnlyFirstCarriesTheReply() {
        enterSideChat()
        val picks = ensureAlbumPhotos()
        // 旁聊里发一句，左滑它当回复对象。
        compose.onNode(hasSetTextAction()).performTextInput("回这一条")
        compose.onNodeWithContentDescription(string(R.string.composer_send)).performClick()
        compose.waitUntil(10_000) { sentItems().fetchSemanticsNodes().isNotEmpty() }
        sentItems()[0].performTouchInput {
            val middle = height / 2f
            swipe(Offset(width * 0.8f, middle), Offset(width * 0.3f, middle), durationMillis = 300)
        }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
        openAlbum()
        compose.waitUntil(10_000) { photos().fetchSemanticsNodes().size >= 2 }
        // 两张能读的照片各选一张。
        photos()[albumIndexOf(picks[0])].performClick()
        photos()[albumIndexOf(picks[1])].performClick()
        sheetSend("attach-send").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty() }
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("quote"), useUnmergedTree = true).fetchSemanticsNodes().size == 1
        }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertDoesNotExist()
    }

    /** 某一张发不出去就停在那里，说清是第几张，附言（草稿）留着。 */
    @Test
    fun sideChatPhotoFailureStopsAndNamesTheNth() {
        enterSideChat()
        val picks = ensureAlbumPhotos()
        val broken = addBrokenPhoto()
        compose.onNode(hasSetTextAction()).performTextInput("这段留着")
        openAlbum()
        compose.waitUntil(10_000) { photos().fetchSemanticsNodes().size >= 2 }
        // 先点坏的那张、再点一张好的：坏的那张失败以后好的不再试。
        photos()[albumIndexOf(broken)].performClick()
        photos()[albumIndexOf(picks[0])].performClick()
        compose.waitForIdle()
        val sentBefore = photoMessages()
        sheetSend("attach-send").performClick()
        // 面板收起就是这次发送收尾了（无论成败）。
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty() }
        check(
            compose.onAllNodes(hasText(string(R.string.photo_failed_at).replace("%1\$d", "1"), substring = true))
                .fetchSemanticsNodes().isNotEmpty(),
        ) { "没说第 1 张发不出去" }
        // 好的那张没再试：照片的假消息一条都没多。草稿留在输入框里。
        check(photoMessages() == sentBefore) { "坏的那张以后还在接着发" }
        compose.onNode(hasText("这段留着")).assertIsDisplayed()
        removePhoto(broken)
    }

    private fun sentItems() = compose.onAllNodes(
        SemanticsMatcher("自己发出去的一条") { node ->
            node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-sent-") == true
        },
        useUnmergedTree = true,
    )

    private fun photoMessages() = compose.onAllNodes(
        SemanticsMatcher("发出去的一张照片") { node ->
            node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-photo-") == true
        },
        useUnmergedTree = true,
    ).fetchSemanticsNodes().size

    private fun openForward() {
        // 12 是长条，气泡比整屏还高，按正中会落到屏幕外；换它上面那条一句话的 11。
        scrollTo(11)
        item(11).performScrollTo()
        bubble(11).performTouchInput { longClick() }
        compose.onNodeWithText(string(R.string.message_forward)).performClick()
        compose.waitForIdle()
    }

    /** 从 Echo 开一个旁聊：发开场白后进去。 */
    private fun enterSideChat() {
        compose.onNodeWithContentDescription(string(R.string.side_chat_new)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 2 }
        compose.onNodeWithTag("side-chat-field").performTextInput("旁聊开场")
        compose.onNodeWithText(string(R.string.composer_send)).performClick()
        // 进了旁聊：顶栏换成旁聊那条，“新旁聊”钮没了，只剩输入框。
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasContentDescription(string(R.string.side_chat_new))).fetchSemanticsNodes().isEmpty() &&
                compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1
        }
    }

    /** 插一张读不出来的“图”（MIME 说是 PNG，内容只有四个字节）：转码必失败。 */
    private fun addBrokenPhoto(): Uri {
        val resolver = instrumentation.targetContext.contentResolver
        val collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "broken.png")
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES)
            // 插入时给的 DATE_ADDED 会被 MediaProvider 冲成当时那一秒，照样给个新值；先后用 albumIndexOf 查。
            put(MediaStore.Images.Media.DATE_ADDED, System.currentTimeMillis() / 1000 + 10)
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }
        val uri = resolver.insert(collection, values) ?: error("相册里放不进坏图")
        resolver.openOutputStream(uri)?.use { it.write(byteArrayOf(1, 2, 3, 4)) }
        values.clear()
        values.put(MediaStore.Images.Media.IS_PENDING, 0)
        resolver.update(uri, values, null, null)
        return uri
    }

    /** 删掉自己插的照片；别的 UID 的行删不掉也不影响这一轮。 */
    private fun removePhoto(uri: Uri) {
        runCatching { instrumentation.targetContext.contentResolver.delete(uri, null, null) }
    }

    private fun androidx.compose.ui.test.SemanticsNodeInteraction.isSelected(): Boolean =
        SemanticsProperties.Selected in fetchSemanticsNode().config
}
