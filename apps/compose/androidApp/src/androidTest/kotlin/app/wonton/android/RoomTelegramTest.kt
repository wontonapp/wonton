package app.wonton.android

import android.Manifest
import android.content.ContentValues
import android.graphics.Bitmap
import android.os.Environment
import android.provider.MediaStore
import android.os.Build
import android.text.StaticLayout
import android.text.TextPaint
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDisplayed
import kotlinx.coroutines.runBlocking
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performScrollToKey
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import android.view.WindowInsets
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.protocol.ImageUpload
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.R
import app.wonton.ui.send
import java.io.File
import java.io.FileOutputStream
import java.text.DateFormat
import java.util.Date
import kotlin.math.abs
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith

/** 假房间里引用别人的那条（`ui_fake_room.rs` 的 `QUOTED_INDEX`）。 */
private const val QUOTED = 19
/** 引用上一条的长中文回复（`ui_fake_room.rs` 的 `LONG_REPLY_INDEX`）。 */
private const val LONG_REPLY = 20
/** 长回复的四段，和假数据一样按空行分段。 */
private val LONG_REPLY_PARAGRAPHS = listOf(
    "没有出问题，插件现在是 running、health 是 healthy、网页返回 200，刚才又确认了一次。",
    "拖得久的原因是我在反复调整页面：每改一次就得重启一次后端才能看到新内容，所以来回花了不少时间。" +
        "这一段故意写得长一些，在手机上会折成好几行，用来看最后几个字会不会被省略号吃掉。",
    "有一件我确实做不到的事：部署后的容器我从这边连不上（内部域名和 8480 端口在这个环境里不可达，也没有 docker 权限），" +
        "所以只能请你帮忙看一眼。",
    "你在 App 里打开「系统」看一眼：数字有在跳、环跟着动，就没问题了。要是空白或者一直转圈，把截图发我，我再接着查。",
)
/** 第 19 条引用内容的原文。 */
private const val QUOTED_TEXT = "🚗\n好久了，有什么问题吗？"

/**
 * Telegram 样式的聊天页（ADR-0109）：左滑回复、点引用跳转和“向下”回去、命令菜单、选项按钮。
 * 只用 Debug 的假核心和它固定的房间，不连家里的服务器。
 */
private const val ECHO_ROOM = "!ui-fake:home.example"
private const val ECHO_NAME = "Echo"

/** 附件区最多放这么多个，清空时够用。 */
private const val MAX_TRAY_ITEMS = 12

@RunWith(AndroidJUnit4::class)
class RoomTelegramTest {
    private val compose = createAndroidComposeRule<UiFakeMainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val permissions: TestRule = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        GrantPermissionRule.grant(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.READ_MEDIA_IMAGES)
    } else {
        GrantPermissionRule.grant()
    }
    private val failureEvidence = object : TestWatcher() {
        override fun failed(error: Throwable, description: Description) {
            screenshot("failure-${description.methodName}")
        }
    }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(permissions).around(compose).around(failureEvidence)

    @Before
    fun openEchoRoom() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        assertNull(core.send(Action.Pages(PageAction.OpenPage(TabRef.Chat))))
        compose.onNodeWithText("Echo").performClick()
        // 假房间的落点在第三条，先把输入框等出来，各用例自己滚到要的那一条。
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    @Test
    fun swipeLeftStartsReply() {
        scrollTo(15)
        item(15).performTouchInput {
            val middle = height / 2f
            swipe(Offset(width * 0.8f, middle), Offset(width * 0.3f, middle), durationMillis = 300)
        }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
        screenshot("swipe-reply")
    }

    @Test
    fun quotedMessageJumpsAndBackButtonReturns() {
        scrollTo(21)
        item(21).performScrollTo()
        // 引用那块本身可点，问主人要不要跳的是它里面的文字。
        compose.onNodeWithText("Fixture message 5", useUnmergedTree = true).performClick()
        compose.waitUntil(10_000) { visible(5) && !visible(21) }
        screenshot("jump-highlight")
        // 离开底部时“向下”先退回跳之前的那条，不直接回最新。
        compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom)).performClick()
        compose.waitUntil(10_000) { visible(21) }
        screenshot("jump-back")
    }

    @Test
    fun quoteBlockHugsItsContent() {
        scrollTo(LONG_REPLY)
        item(LONG_REPLY).performScrollTo()
        val quote = compose.onNode(
            hasTestTag("quote") and hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        )
        val box = quote.fetchSemanticsNode().boundsInRoot
        val name = compose.onNode(
            hasText("Owner") and hasAnyAncestor(hasTestTag("quote")) and
                hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        ).fetchSemanticsNode().boundsInRoot
        val summary = compose.onNode(
            hasText(QUOTED_TEXT.trim()) and hasAnyAncestor(hasTestTag("quote")) and
                hasAnyAncestor(hasTestTag("item-ui-fake-$LONG_REPLY")),
            useUnmergedTree = true,
        ).fetchSemanticsNode().boundsInRoot
        val slack = with(compose.density) { 12.dp.toPx() }
        // 引用块只占名字加内容本身那么高：内容就是两行，多出来的就是被拉高的。
        check(summary.bottom >= name.bottom) { "引用块的正文跑到名字上面：${summary.bottom} < ${name.bottom}" }
        check(box.height <= name.height + summary.height + slack) {
            "引用块被拉高：${box.height} > 名字 ${name.height} + 内容 ${summary.height} + $slack"
        }
        check(box.bottom - summary.bottom <= slack) { "引用块底下多出 ${box.bottom - summary.bottom}" }
    }

    @Test
    fun timeNeverCoversMessageText() {
        scrollTo(LONG_REPLY)
        item(LONG_REPLY).performScrollTo()
        fun timeNode(index: Int) = compose.onAllNodes(
            hasTestTag("messageTime") and hasAnyAncestor(hasTestTag("item-ui-fake-$index")),
            useUnmergedTree = true,
        ).fetchSemanticsNodes().singleOrNull()
        for (index in 18..23) {
            // 一条一条滚进来：离屏的条目语义在、框是空的。
            scrollTo(index)
            compose.waitForIdle()
            var time = timeNode(index)
            if (time != null && time.boundsInRoot.width == 0f) {
                compose.onNodeWithTag("room-timeline").performTouchInput { swipeUp() }
                compose.waitForIdle()
                time = timeNode(index)
            }
            if (time == null || time.boundsInRoot.width == 0f) continue
            val box = time.boundsInRoot
            val bubble = bubble(index).fetchSemanticsNode().boundsInRoot
            check(bubble.contains(box.topLeft) && bubble.contains(box.bottomRight - Offset(1f, 1f))) {
                "时间跑出气泡：$box 不在 $bubble"
            }
            val nodes = bodyNodes(index).sortedBy { it.boundsInRoot.top }
            // 末尾那段透明占位从最后一个普通空格开始，藏在它后面的字不算正文；卡片条目的末节点是按钮，不带占位。
            val tail = layoutOf(nodes.last())!!.layoutInput.text.text
            val keep = if (tail.endsWith('\u00A0')) tail.lastIndexOf(' ') else tail.length
            val lines = nodes.flatMap { lineBoxes(it, if (it === nodes.last()) tail.length - keep else 0) }
            check(lines.isNotEmpty()) { "第 $index 条没有正文行框：'$tail'" }
            val hit = lines.firstOrNull { it.overlaps(box) }
            check(hit == null) {
                "时间压在第 $index 条的字上：$box 与 $hit；气泡 $bubble；" +
                    nodes.joinToString(" | ") { "${it.boundsInRoot} '${layoutOf(it)!!.layoutInput.text.text}'" }
            }
        }
    }

    @Test
    fun longReplyKeepsEveryParagraphWhole() {
        scrollTo(LONG_REPLY)
        item(LONG_REPLY).performScrollTo()
        val nodes = bodyNodes(LONG_REPLY).sortedBy { it.boundsInRoot.top }
        check(nodes.size == LONG_REPLY_PARAGRAPHS.size) { "段落少了：${nodes.size} 段" }
        for ((node, expected) in nodes.zip(LONG_REPLY_PARAGRAPHS)) {
            val layout = layoutOf(node) ?: continue
            val drawn = layout.layoutInput.text.text
            check(drawn.startsWith(expected)) { "这一段的文字对不上：'$drawn'" }
            // 语义里要留着最后一句，读屏和复制拿到的都是全文。
            check(drawn.contains(expected.takeLast(6))) { "这一段少了结尾：'${drawn.takeLast(12)}'" }
            // 换行数按同样的宽度重新排一遍；少一行就是被截断了。
            val paint = TextPaint().apply {
                textSize = with(compose.density) { layout.layoutInput.style.fontSize.toPx() }
            }
            val needs = StaticLayout.Builder.obtain(expected, 0, expected.length, paint, node.size.width)
                .build().lineCount
            check(layout.lineCount >= needs - 1) { "这一段被截断：画了 ${layout.lineCount} 行，要 $needs 行" }
        }
        screenshot("long-reply")
    }

    /**
     * 回形针打开相册网格，点两张以后底下换成附言条，写了附言点发送：面板收起、附件区不留东西，
     * 时间线里出现带这张附言的消息。相册里没有照片时先放两张进去；照片只在本机相册里。
     * 附件区存在盘上、进程之间留着：先把剩下的拿掉。
     */
    @Test
    fun attachSheetSendsPhotosWithNote() {
        ensureAlbumPhotos()
        clearTray()
        compose.onNodeWithContentDescription(string(R.string.composer_attach)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("album-grid")).fetchSemanticsNodes().size == 1 }
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("album-photo")).fetchSemanticsNodes().size >= 2 }
        compose.onAllNodes(hasTestTag("album-photo"))[0].performClick()
        compose.onAllNodes(hasTestTag("album-photo"))[1].performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isNotEmpty() }
        val field = compose.onNode(hasSetTextAction() and hasAnyAncestor(hasTestTag("attach-send")))
        field.performTextInput("两张都发")
        compose.waitForIdle()
        val shown = compose.onAllNodes(hasText("两张都发", substring = true)).fetchSemanticsNodes().size
        check(shown > 0) { "输入框里没进去：${field.fetchSemanticsNode().config}" }
        screenshot("attach-sheet")
        sheetSend("attach-send").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty() }
        runCatching { compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom)).performClick() }
        compose.waitUntil(10_000) { compose.onAllNodes(hasText("两张都发")).fetchSemanticsNodes().isNotEmpty() }
        screenshot("attach-sent")
        check(trayTiles() == 0) { "附件区里还剩着照片" }
        compose.onNode(hasSetTextAction()).assertIsDisplayed()
    }

    /**
     * 附件区里有一张没传上去的照片时点发送：核心拒收，字、附件和回复对象都得留着，原因说出来。
     * 现在发送一律当成功，字丢了、回复也收了。
     */
    @Test
    fun rejectedSendKeepsDraftAndReply() {
        val core = (compose.activity.application as WontonApplication).core.getOrThrow()
        ensureAlbumPhotos()
        clearTray()
        compose.onNodeWithContentDescription(string(R.string.composer_attach)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("album-photo")).fetchSemanticsNodes().isNotEmpty() }
        compose.onAllNodes(hasTestTag("album-photo"))[0].performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isNotEmpty() }
        sheetSend("attach-send").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("attach-send")).fetchSemanticsNodes().isEmpty() }
        compose.waitUntil(10_000) { trayTiles() == 1 }
        // 回一条，再把附件标成上传失败。
        scrollTo(15)
        item(15).performTouchInput {
            val middle = height / 2f
            swipe(Offset(width * 0.8f, middle), Offset(width * 0.3f, middle), durationMillis = 300)
        }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
        compose.onNode(hasSetTextAction()).performTextInput("先发这句")
        core.failAttachmentForTest(TimelineTarget(ECHO_ROOM, null), trayFirstId(core))
        compose.waitForIdle()
        compose.onNodeWithContentDescription(string(R.string.composer_send)).performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasText("Retry or remove it first", substring = true)).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNode(hasText("先发这句")).assertIsDisplayed()
        check(trayTiles() == 1) { "附件被吃掉了" }
        compose.onNodeWithContentDescription(string(R.string.message_reply_cancel)).assertIsDisplayed()
    }

    /**
     * 转发面板：搜索能收窄列表，勾上两个房间、写上附言后一起发出去。每个房间都先到附言，
     * 再到红色的同文消息；勾了好几个就留在原房间。
     */
    @Test
    fun forwardPickerSendsNoteBeforeMessageToEveryTarget() {
        // 12 是长条，气泡比整屏还高，按正中会落到屏幕外；换它上面那条一句话的 11。
        scrollTo(11)
        item(11).performScrollTo()
        bubble(11).performTouchInput { longClick() }
        compose.onNodeWithText(string(R.string.message_forward)).performClick()
        compose.waitForIdle()
        compose.waitUntil(10_000) { forwardRows() == 2 }
        sheetSend("forward-send").assertIsNotEnabled()
        compose.onNodeWithTag("forward-search").performTextInput("Fam")
        compose.waitUntil(10_000) { forwardRows() == 1 }
        compose.onNode(hasTestTag("forward-row") and hasText("Family")).performClick()
        compose.onNodeWithTag("forward-search").performTextClearance()
        compose.waitUntil(10_000) { forwardRows() == 2 }
        compose.onNode(hasTestTag("forward-row") and hasText("Notes")).performClick()
        compose.onNode(hasSetTextAction() and hasAnyAncestor(hasTestTag("forward-send")))
            .performTextInput("都看一下")
        screenshot("forward-sheet")
        sheetSend("forward-send").performClick()
        // 勾了两个：留在原房间，两条消息都在，谁也没跳走。
        compose.waitForIdle()
        check(
            compose.onAllNodes(hasText(ECHO_NAME) and hasAnyAncestor(hasTestTag("room-bar")))
                .fetchSemanticsNodes().isNotEmpty(),
        ) { "原房间跳走了" }
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.waitForIdle()
        if (hasBack()) compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        // 到了聊天列表，依次进 Family 和 Notes，两边都得有附言和转发来的那条。
        compose.onNodeWithText("Family").performClick()
        compose.waitUntil(10_000) { forwardedInto("Family", "ui-fake-forwarded-24") }
        check(compose.onAllNodes(hasText("都看一下")).fetchSemanticsNodes().isNotEmpty()) { "Family 没收到附言" }
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.waitForIdle()
        if (hasBack()) compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.onNodeWithText("Notes").performClick()
        compose.waitUntil(10_000) { forwardedInto("Notes", "ui-fake-forwarded-24") }
        check(compose.onAllNodes(hasText("都看一下")).fetchSemanticsNodes().isNotEmpty()) { "Notes 没收到附言" }
    }

    /** 只勾一个时发完跟过去：那个房间打开，附言和转发都在。 */
    @Test
    fun forwardToSingleTargetOpensIt() {
        scrollTo(11)
        item(11).performScrollTo()
        bubble(11).performTouchInput { longClick() }
        compose.onNodeWithText(string(R.string.message_forward)).performClick()
        compose.waitUntil(10_000) { forwardRows() == 2 }
        compose.onNode(hasTestTag("forward-row") and hasText("Family")).performClick()
        sheetSend("forward-send").performClick()
        compose.waitUntil(10_000) { forwardedInto("Family", "ui-fake-forwarded-24") }
    }

    private fun forwardRows() = compose.onAllNodes(hasTestTag("forward-row")).fetchSemanticsNodes().size

    /** 面板附言条上的发送键：输入框底下那个，和房间里被面板挡住的发送键区分开。 */
    private fun sheetSend(tag: String) =
        compose.onNode(hasContentDescription(string(R.string.composer_send)) and hasAnyAncestor(hasTestTag(tag)))

    private fun hasBack() =
        compose.onAllNodes(hasContentDescription(string(R.string.design_back))).fetchSemanticsNodes().isNotEmpty()

    /** 房间打开了（顶栏是它的名字），并且时间线里有那条转发来的。房间刚开时落在老消息中间，点“向下”回最新。 */
    private fun forwardedInto(room: String, id: String): Boolean {
        val bar = compose.onAllNodes(
            hasText(room) and hasAnyAncestor(hasTestTag("room-bar")),
            useUnmergedTree = true,
        ).fetchSemanticsNodes()
        val open = bar.isNotEmpty()
        fun seen() = compose.onAllNodes(hasTestTag("item-$id")).fetchSemanticsNodes().isNotEmpty()
        if (open && !seen()) {
            runCatching { compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom)).performClick() }
        }
        return open && seen()
    }

    private fun trayIds(core: ClientCore): List<String> = runBlocking {
        core.subscribeComposer(TimelineTarget(ECHO_ROOM, null)).use { sub ->
            sub.next()!!.attachments.map { it.id }
        }
    }

    private fun trayFirstId(core: ClientCore): String = trayIds(core).first()

    /**
     * 把附件区里留着的附件一个个拿掉：一排照片会横着滚，先把行滚回开头再点第一个拿掉的按钮，
     * 否则点到的是滚出屏幕的格子。
     */
    private fun clearTray() {
        val prefix = string(R.string.composer_attachment_remove).substringBefore('%')
        val removes = SemanticsMatcher("拿掉附件") { node ->
            node.config.getOrNull(SemanticsProperties.ContentDescription).orEmpty().any { it.startsWith(prefix) }
        }
        repeat(MAX_TRAY_ITEMS) {
            if (compose.onAllNodes(removes, useUnmergedTree = true).fetchSemanticsNodes().isEmpty()) return
            compose.onNodeWithTag("composerTray").performScrollToIndex(0)
            compose.waitForIdle()
            compose.onAllNodes(removes, useUnmergedTree = true)[0].performClick()
            compose.waitForIdle()
        }
        check(compose.onAllNodes(removes, useUnmergedTree = true).fetchSemanticsNodes().isEmpty()) {
            "附件区没清空"
        }
    }

    private fun trayTiles(): Int = compose.onAllNodes(
        SemanticsMatcher("附件区里的照片") { node ->
            node.config.getOrNull(SemanticsProperties.ContentDescription).orEmpty()
                .any { it == "photo.png" || it == "photo.jpg" }
        },
        useUnmergedTree = true,
    ).fetchSemanticsNodes().size

    /** 相册里照片不够两张时先放两张真照片进去，够了两张就不再加，多跑几次也不堆。 */
    private fun ensureAlbumPhotos() {
        val resolver = instrumentation.targetContext.contentResolver
        val collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        val existing = resolver.query(collection, arrayOf(MediaStore.Images.Media._ID), null, null, null)
            ?.use { it.count } ?: 0
        repeat((2 - existing).coerceAtLeast(0)) { index ->
            val values = ContentValues().apply {
                put(MediaStore.Images.Media.DISPLAY_NAME, "pick-$index.png")
                put(MediaStore.Images.Media.MIME_TYPE, "image/png")
                put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES)
                put(MediaStore.Images.Media.IS_PENDING, 1)
            }
            val uri = resolver.insert(collection, values) ?: error("相册里放不进照片")
            resolver.openOutputStream(uri)?.use { out ->
                Bitmap.createBitmap(320, 320, Bitmap.Config.ARGB_8888)
                    .apply { eraseColor(if (index % 2 == 0) 0xFF8A6E4B.toInt() else 0xFF3F5F4F.toInt()) }
                    .compress(Bitmap.CompressFormat.PNG, 100, out)
            }
            values.clear()
            values.put(MediaStore.Images.Media.IS_PENDING, 0)
            resolver.update(uri, values, null, null)
        }
    }

    @Test
    fun commandMenuListsBotCommandsAndSends() {
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.onNodeWithTag("command-help").assertIsDisplayed()
        compose.onNodeWithTag("command-model").assertIsDisplayed()
        settleKeyboard()
        screenshot("command-menu")
        compose.onNodeWithTag("command-new").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText("/new", substring = true, useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        settleKeyboard()
        screenshot("command-sent")
    }

    @Test
    fun choiceAnswerSelectsItAndDisablesTheOther() {
        scrollTo(22)
        val approve = choice("✅")
        val deny = choice("❌")
        approve.assertIsEnabled()
        // 表情先画出来，按钮行不比气泡宽。
        compose.onAllNodesWithText("✅", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        val rowWidth = compose.onNode(
            hasTestTag("choices-row"),
            useUnmergedTree = true,
        ).fetchSemanticsNode().boundsInRoot.width
        val bubbleWidth = bubble(22).fetchSemanticsNode().boundsInRoot.width
        check(rowWidth <= bubbleWidth + 0.5f) { "按钮行比气泡宽：$rowWidth > $bubbleWidth" }
        // 表情和字在同一行：两个按钮一样高，说明不比同行的表情高多少（折行会翻倍）。
        val heights = listOf(approve, deny).map { it.fetchSemanticsNode().boundsInRoot.height }
        check(abs(heights[0] - heights[1]) <= 1f) { "两个按钮不一样高：$heights" }
        val emoji = compose.onAllNodesWithText("✅", useUnmergedTree = true).fetchSemanticsNodes().single()
        val label = compose.onAllNodesWithText("approve once", useUnmergedTree = true).fetchSemanticsNodes().single()
        check(label.boundsInRoot.height <= emoji.boundsInRoot.height * 1.5f) {
            "说明折行了：${label.boundsInRoot.height} > ${emoji.boundsInRoot.height}"
        }
        check(label.boundsInRoot.top < approve.fetchSemanticsNode().boundsInRoot.bottom)
        // 表情和字之间留一点缝（s8），别贴成一个词，也别拉成两截。
        val want = with(compose.density) { 8.dp.toPx() }
        val gap = label.boundsInRoot.left - emoji.boundsInRoot.right
        check(abs(gap - want) <= 2f) { "表情和字之间是 $gap，不是 $want" }
        screenshot("choices")
        approve.performClick()
        compose.waitUntil(10_000) { !deny.isEnabled() }
        choice("✅").assertIsSelected()
        deny.assertIsNotEnabled()
        screenshot("choice-answered")
    }

    @Test
    fun groupHeadAndTailShapes() {
        openFamily()
        // 群里的别人有头像，私聊没有。
        compose.waitUntil(10_000) { avatars().isNotEmpty() }
        scrollTo(8)
        item(8).performScrollTo()
        item(8).assertIsDisplayed()
        screenshot("group-head")
        scrollTo(10)
        item(10).performScrollTo()
        item(10).assertIsDisplayed()
        // 组尾：尾巴那条和组内上一条的气泡左边对齐，头像挂在组尾。
        val head = bubble(9).fetchSemanticsNode().boundsInRoot.left
        check(bubble(10).fetchSemanticsNode().boundsInRoot.left == head) { "组内气泡左边没对齐" }
        screenshot("group-tail")
    }

    @Test
    fun groupShotShowsNameAndAvatar() {
        openFamily()
        scrollTo(8)
        item(8).performScrollTo()
        scrollTo(10)
        item(10).assertIsDisplayed()
        compose.waitUntil(10_000) { avatars().isNotEmpty() }
        screenshot("group")
    }

    @Test
    fun directRoomHasNoSenderAvatar() {
        scrollTo(8)
        item(8).performScrollTo()
        compose.onAllNodes(hasTestTag("sender-avatar"), useUnmergedTree = true).assertCountEquals(0)
    }

    @Test
    fun groupTopKeepsFirstBubbleBelowTheBar() {
        openFamily()
        // 23 是最早的一条，翻到最上面就是它。
        scrollTo(23)
        repeat(3) {
            compose.onNodeWithTag("room-timeline").performTouchInput {
                swipe(Offset(centerX, centerY * 0.3f), Offset(centerX, centerY * 1.9f), durationMillis = 400)
            }
            compose.waitForIdle()
        }
        bubble(23).assertIsDisplayed()
        // 最早那条气泡的顶边还在顶栏下面，没被顶栏拿走。
        val first = bubble(23).fetchSemanticsNode().boundsInRoot
        // 顶栏和它下面那行（群里的 AI 等）都不能压在第一条上。
        val bar = compose.onNodeWithTag("room-bar").fetchSemanticsNode().boundsInRoot.bottom
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot.bottom
        val limit = maxOf(bar, header)
        check(first.top >= limit) { "第一条被顶栏盖住：${first.top} < $limit" }
    }

    @Test
    fun datePillFadesAfterScrollingStops() {
        scrollTo(15)
        compose.onNodeWithTag("room-timeline").performTouchInput {
            swipe(Offset(centerX, height * 0.7f), Offset(centerX, height * 0.2f), durationMillis = 300)
        }
        compose.waitUntil(5_000) { pillVisible() }
        // 停下 motion.datePillHold（1 秒）后淡出，等不到就是没淡出。
        compose.waitUntil(10_000) { !pillVisible() }
    }

    @Test
    fun daySeparatorRidesAboveTheFirstOfADay() {
        // 第 11 条是后一天的第一条：分隔夹在它和上面第 10 条之间，写它的日期。分隔翻到附近才进屏幕，撞上为止。
        val (separators, bubbles) = scrollToFirstSeparator()
        for (separator in separators) separatorIsAboveTheOneItNames(separator, bubbles)
        noSeparatorBetweenTheSameDay()
        // 屏幕上往下是越来越新：分隔自上而下的日期严格变晚。
        val dates = separators.sortedBy { it.boundsInRoot.top }
            .map { checkNotNull(DateFormat.getDateInstance(DateFormat.MEDIUM).parse(texts(it))) }
        for (position in 1 until dates.size) {
            check(dates[position].after(dates[position - 1])) { "日期没随消息往下变晚：$dates" }
        }
        // 列表最顶上那条的上面也有分隔，写它自己的日期。
        scrollTo(0)
        val firstOfList = bubble(0).fetchSemanticsNode()
        val aboveFirst = compose.onAllNodes(hasTestTag("day-separator"), useUnmergedTree = true)
            .fetchSemanticsNodes()
            .lastOrNull { it.boundsInRoot.bottom <= firstOfList.boundsInRoot.top + 1f }
        checkNotNull(aboveFirst) { "列表最顶上那条的上面没有日期分隔" }
        check(texts(aboveFirst) == fixtureDate(0)) { "最顶上的分隔不是第一条的日期：${texts(aboveFirst)}" }
        // 这条分隔和它两边的消息一起进画面，留一张给人看。
        scrollTo(11)
        compose.waitUntil(10_000) { visible(10) && visible(11) }
        screenshot("day-separator")
    }

    /** 翻到有分隔的地方，返回分隔和气泡（按屏幕从上到下）。 */
    private fun scrollToFirstSeparator(): Pair<List<SemanticsNode>, List<SemanticsNode>> {
        var separators = emptyList<SemanticsNode>()
        var seen = ""
        for (index in listOf(11, 12, 10, 13, 9)) {
            scrollTo(index)
            compose.waitForIdle()
            separators = compose.onAllNodes(hasTestTag("day-separator"), useUnmergedTree = true)
                .fetchSemanticsNodes()
            seen += "$index:${separators.size} "
            if (separators.isNotEmpty()) break
        }
        check(separators.isNotEmpty()) { "没画日期分隔：$seen" }
        return separators to compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true)
            .fetchSemanticsNodes().sortedBy { it.boundsInRoot.top }
    }

    /** 分隔夹在前一天最后一条和后一天第一条之间：下面紧挨的那条是它写的这一天，上面那条是更早的一天。 */
    private fun separatorIsAboveTheOneItNames(separator: SemanticsNode, bubbles: List<SemanticsNode>) {
        val mark = separator.boundsInRoot
        check(bubbles.none { it.boundsInRoot.overlaps(mark) }) { "气泡压在日期分隔上：$mark" }
        val below = bubbles.firstOrNull { it.boundsInRoot.top >= mark.bottom - 1f }
        val belowIndex = checkNotNull(below?.let(::fixtureIndexOf)) { "日期分隔下面认不出气泡：$mark" }
        check(texts(separator) == fixtureDate(belowIndex)) {
            "日期分隔不是它下面那条的日期：${texts(separator)} vs ${fixtureDate(belowIndex)}（第 $belowIndex 条）"
        }
        val above = bubbles.lastOrNull { it.boundsInRoot.bottom <= mark.top + 1f }
        val aboveIndex = above?.let(::fixtureIndexOf) ?: return
        check(fixtureDate(aboveIndex) != fixtureDate(belowIndex)) {
            "同一天（第 $aboveIndex / $belowIndex 条）的气泡之间夹了日期分隔"
        }
        val order = "分隔上面第 $aboveIndex 条 ${above!!.boundsInRoot}，" +
            "下面第 $belowIndex 条 ${below.boundsInRoot}，分隔 $mark"
        check(fixtureDate(aboveIndex) < fixtureDate(belowIndex)) { "上面的日期不早于下面：$order" }
    }

    /** 同一天相邻的两条之间没有分隔。 */
    private fun noSeparatorBetweenTheSameDay() {
        val bubbles = compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true)
            .fetchSemanticsNodes().sortedBy { it.boundsInRoot.top }
        val separators = compose.onAllNodes(hasTestTag("day-separator"), useUnmergedTree = true)
            .fetchSemanticsNodes()
        val sameDay = bubbles.zipWithNext().filter { (above, below) ->
            val a = fixtureIndexOf(above)
            val b = fixtureIndexOf(below)
            a != null && b != null && fixtureDate(a) == fixtureDate(b)
        }
        for ((above, below) in sameDay) {
            val between = separators.filter {
                it.boundsInRoot.top >= above.boundsInRoot.bottom - 1f &&
                    it.boundsInRoot.bottom <= below.boundsInRoot.top + 1f
            }
            val pair = "${fixtureIndexOf(above)} / ${fixtureIndexOf(below)}"
            check(between.isEmpty()) { "同一天的第 $pair 条之间夹了日期分隔" }
        }
    }

    /** 气泡所在消息的序号：按外层条目的标签认，引用的文字里也会出现“Fixture message N”。 */
    private fun fixtureIndexOf(bubble: SemanticsNode): Int? {
        val items = compose.onAllNodes(
            SemanticsMatcher("是消息条目") {
                it.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-") == true
            },
            useUnmergedTree = true,
        ).fetchSemanticsNodes()
        val owner = items.firstOrNull { it.boundsInRoot.contains(bubble.boundsInRoot.center) }
        return owner?.config?.getOrNull(SemanticsProperties.TestTag)?.removePrefix("item-ui-fake-")?.toInt()
    }

    /** 节点自己和它下面所有子孙的文字；气泡的标题挂在子节点上。 */
    private fun texts(node: SemanticsNode): String =
        node.config.getOrNull(SemanticsProperties.Text).orEmpty().joinToString() +
            node.children.joinToString("") { texts(it) }

    /** 假数据第 `index` 条的日期：第 0 条早两天，11 起是后一天，其余同一天，每条隔一分钟（同 `ui_fake_room.rs`）。 */
    private fun fixtureDate(index: Int): String {
        val at = 1_792_000_000_000L + index * 60_000L +
            if (index >= 11) 86_400_000L else 0L -
            if (index == 0) 172_800_000L else 0L
        return DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(at))
    }

    /** 回到房间列表再进群；用例都在同一个活动里连着跑。 */
    private fun openFamily() {
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.onNodeWithText("Family").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    private fun avatars() =
        compose.onAllNodes(hasTestTag("sender-avatar"), useUnmergedTree = true).fetchSemanticsNodes()

    /** 键盘动画比布局慢一拍，等表面画上来再截，不然截到的还是刚才那一帧。 */
    private fun settleKeyboard() {
        compose.waitUntil(10_000) { imeHeight() > 0 }
        compose.waitForIdle()
        Thread.sleep(300)
    }

    private fun pillVisible() = compose.onAllNodes(hasTestTag("date-pill"), useUnmergedTree = true)
        .fetchSemanticsNodes().isNotEmpty()

    @Test
    fun commandMenuKeepsBarAndComposerInPlace() {
        val statusBar = statusBarHeight()
        compose.onNodeWithContentDescription(string(R.string.composer_commands)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("command-help")).fetchSemanticsNodes().isNotEmpty() }
        settleKeyboard()
        // 键盘起来以后顶栏原地不动，输入框压在键盘上沿，时间线还在。
        // 顶栏的盒子从窗口顶起，看它里面的标题：标题还在状态栏下面就是没被顶走。
        val title = compose.onAllNodesWithText("Echo", useUnmergedTree = true)
            .fetchSemanticsNodes().first().boundsInRoot
        check(title.top >= statusBar - 1f) { "顶栏被顶到状态栏底下：${title.top} < $statusBar" }
        val bar = compose.onNodeWithTag("room-bar").fetchSemanticsNode().boundsInRoot
        check(bar.bottom >= statusBar) { "顶栏整条被顶走：${bar.bottom} < $statusBar" }
        val input = compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().single().boundsInRoot
        val keyboardTop = compose.activity.window.decorView.height - imeHeight()
        // 输入区整条贴着键盘上沿，输入框在它最下沿（自己留了一点内边距）。
        val block = compose.onNodeWithTag("composer-block").fetchSemanticsNode().boundsInRoot
        val gap = keyboardTop - block.bottom
        check(abs(gap) <= 2f) { "输入区没贴着键盘上沿：差 $gap" }
        check(input.bottom <= block.bottom + 0.5f) { "输入框掉出输入区：${input.bottom} > ${block.bottom}" }
        // 命令多的时候输入区顶到顶栏下沿为止，顶栏本身不能被它盖住。
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot
        check(block.top >= header.bottom - 1f) { "输入区盖住顶栏：${block.top} < ${header.bottom}" }
        check(compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot.height > 0f)
        // 菜单盖住输入框上方，“向下”钮和菜单叠在状态栏边，菜单开着时不画它。
        compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom)).assertIsNotDisplayed()
        screenshot("command-menu")
    }

    @Test
    fun pluginStatusRowCoversNoBubble() {
        // 群里 Bot 掉线的那行（假房间里是 Hermes）自己占一条，时间线让开它，气泡不压在它上面。
        openFamily()
        compose.onNodeWithText("Hermes", useUnmergedTree = true).assertIsDisplayed()
        val header = compose.onNodeWithTag("room-header").fetchSemanticsNode().boundsInRoot
        val bubbles = compose.onAllNodes(hasTestTag("bubble"), useUnmergedTree = true).fetchSemanticsNodes()
        check(bubbles.isNotEmpty())
        for (bubble in bubbles) {
            val box = bubble.boundsInRoot
            check(!box.overlaps(header)) { "气泡压在状态行上：$box vs $header" }
        }
    }

    /** 这条消息的中心在时间线视口里。跳转过后 `isDisplayed` 会因为列表项没 placed 而恒假。 */
    private fun visible(index: Int): Boolean {
        val list = compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot
        val center = compose.onAllNodes(hasTestTag("item-ui-fake-$index")).fetchSemanticsNodes().singleOrNull()
            ?.boundsInRoot?.center?.y ?: return false
        return center in list.top..list.bottom
    }

    private fun item(index: Int) = compose.onNodeWithTag("item-ui-fake-$index")

    private fun bubble(index: Int) = compose.onNode(
        hasTestTag("bubble") and hasAnyAncestor(hasTestTag("item-ui-fake-$index")),
        useUnmergedTree = true,
    )

    /** 列表倒着排，按 key 滚过去；条目还没组合出来，`performScrollTo` 找不到。 */
    private fun scrollTo(index: Int) {
        // 时间线是发完再组合的：列表里一条都没有时按 key 滚会报找不到。
        compose.waitUntil(10_000) {
            compose.onAllNodes(SemanticsMatcher("时间线条目") { node ->
                node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-") == true
            }, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("room-timeline").performScrollToKey("ui-fake-$index")
    }

    /** 有 `Disabled` 语义就不算能用。 */
    private fun SemanticsNodeInteraction.isEnabled(): Boolean =
        SemanticsProperties.Disabled !in fetchSemanticsNode().config

    private fun layoutOf(node: SemanticsNode): TextLayoutResult? {
        val action = node.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action ?: return null
        val results = mutableListOf<TextLayoutResult>()
        return if (action.invoke(results)) results.firstOrNull() else null
    }

    /** 这条消息的正文文字节点，跳过引用块和脚注。 */
    private fun bodyNodes(index: Int): List<SemanticsNode> = compose.onAllNodes(
        hasAnyAncestor(hasTestTag("item-ui-fake-$index")) and
            SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult) and
            !hasAnyAncestor(hasTestTag("quote")) and
            !hasTestTag("messageTime"),
        useUnmergedTree = true,
    ).fetchSemanticsNodes()

    /** 节点的每个字行占的框，`reserve` 个字符是末尾的透明占位，不算正文。 */
    private fun lineBoxes(node: SemanticsNode, reserve: Int): List<Rect> {
        val layout = layoutOf(node) ?: return emptyList()
        val origin = node.boundsInRoot.topLeft
        val length = layout.layoutInput.text.length
        val cut = (length - reserve).coerceAtLeast(0)
        return (0 until layout.lineCount).mapNotNull { line ->
            var box: Rect? = null
            for (index in layout.getLineStart(line) until minOf(layout.getLineEnd(line), cut)) {
                val glyph = layout.getBoundingBox(index)
                if (glyph.width <= 0f) continue
                box = box?.expandTo(glyph) ?: glyph
            }
            box?.translate(origin)
        }
    }

    /** 两个框并成一个，包住两边。 */
    private fun Rect.expandTo(other: Rect) = Rect(
        minOf(left, other.left),
        minOf(top, other.top),
        maxOf(right, other.right),
        maxOf(bottom, other.bottom),
    )

    /** 选项的选中和停用语义在内层那行，标签在外层，按祖先找；选中与否两种状态都有 `Selected`。 */
    private fun choice(key: String) = compose.onNode(
        hasAnyAncestor(hasTestTag("choice-$key")) and
            SemanticsMatcher.keyIsDefined(SemanticsProperties.Selected),
        useUnmergedTree = true,
    )

    private fun string(id: Int) = compose.activity.getString(id)

    /** 状态栏和键盘的高度（像素），从窗口的实时 insets 取。 */
    private fun statusBarHeight(): Int =
        compose.activity.window.decorView.rootWindowInsets.getInsets(WindowInsets.Type.statusBars()).top

    private fun imeHeight(): Int =
        compose.activity.window.decorView.rootWindowInsets.getInsets(WindowInsets.Type.ime()).bottom

    private fun screenshot(name: String) {
        // 滚过以后浮动日期胶囊还在，等它淡出再拍，不然截到的是它盖在字上的样子；失败现场可能已经停了，等不到也照拍。
        runCatching { compose.waitUntil(5_000) { !pillVisible() } }
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "telegram")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
}
