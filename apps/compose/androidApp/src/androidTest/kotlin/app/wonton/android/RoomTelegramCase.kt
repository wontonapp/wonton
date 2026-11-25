package app.wonton.android

import android.Manifest
import android.content.ContentUris
import android.content.ContentValues
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.view.WindowInsets
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performScrollToKey
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.compose.ui.text.TextLayoutResult
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.R
import app.wonton.ui.send
import java.io.File
import java.io.FileOutputStream
import java.text.DateFormat
import java.util.Date
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith

/** 假房间里引用别人的那条（`ui_fake_room.rs` 的 `QUOTED_INDEX`）。 */
internal const val QUOTED = 19

/** 引用上一条的长中文回复（`ui_fake_room.rs` 的 `LONG_REPLY_INDEX`）。 */
internal const val LONG_REPLY = 20

/** 长回复的四段，和假数据一样按空行分段。 */
internal val LONG_REPLY_PARAGRAPHS = listOf(
    "没有出问题，插件现在是 running、health 是 healthy、网页返回 200，刚才又确认了一次。",
    "拖得久的原因是我在反复调整页面：每改一次就得重启一次后端才能看到新内容，所以来回花了不少时间。" +
        "这一段故意写得长一些，在手机上会折成好几行，用来看最后几个字会不会被省略号吃掉。",
    "有一件我确实做不到的事：部署后的容器我从这边连不上（内部域名和 8480 端口在这个环境里不可达，也没有 docker 权限），" +
        "所以只能请你帮忙看一眼。",
    "你在 App 里打开「系统」看一眼：数字有在跳、环跟着动，就没问题了。要是空白或者一直转圈，把截图发我，我再接着查。",
)

/** 第 19 条引用内容的原文。 */
internal const val QUOTED_TEXT = "🚗\n好久了，有什么问题吗？"

internal const val ECHO_ROOM = "!ui-fake:home.example"
internal const val ECHO_NAME = "Echo"

/** 附件区最多放这么多个，清空时够用。 */
internal const val MAX_TRAY_ITEMS = 12

/**
 * Telegram 样式的聊天页（ADR-0109）用例的公共夹具：只用 Debug 的假核心和它固定的房间，不连家里的服务器。
 * 按负责的面分给 `RoomTelegram*Test`（回复、时间线、选项、失败、头像、顶栏、待确认等）、`RoomCommandMenuTest`（命令菜单）
 * 和 `RoomAttachPanelTest`（附件、转发面板）、`RoomEditTest`（编辑消息）继承。
 */
@RunWith(AndroidJUnit4::class)
abstract class RoomTelegramCase {
    protected val compose = createAndroidComposeRule<UiFakeMainActivity>()
    protected val instrumentation = InstrumentationRegistry.getInstrumentation()
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

    /** 这条消息的中心在时间线视口里。跳转过后 `isDisplayed` 会因为列表项没 placed 而恒假。 */
    protected fun visible(index: Int): Boolean {
        val list = compose.onNodeWithTag("room-timeline").fetchSemanticsNode().boundsInRoot
        val center = compose.onAllNodes(hasTestTag("item-ui-fake-$index")).fetchSemanticsNodes().singleOrNull()
            ?.boundsInRoot?.center?.y ?: return false
        return center in list.top..list.bottom
    }

    protected fun item(index: Int) = compose.onNodeWithTag("item-ui-fake-$index")

    protected fun bubble(index: Int) = compose.onNode(
        hasTestTag("bubble") and hasAnyAncestor(hasTestTag("item-ui-fake-$index")),
        useUnmergedTree = true,
    )

    /** 列表倒着排，按 key 滚过去；条目还没组合出来，`performScrollTo` 找不到。 */
    protected fun scrollTo(index: Int) {
        // 时间线是发完再组合的：列表里一条都没有时按 key 滚会报找不到。
        compose.waitUntil(10_000) {
            compose.onAllNodes(SemanticsMatcher("时间线条目") { node ->
                node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-ui-fake-") == true
            }, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("room-timeline").performScrollToKey("ui-fake-$index")
    }

    /** 有 `Disabled` 语义就不算能用。 */
    protected fun SemanticsNodeInteraction.isEnabled(): Boolean =
        SemanticsProperties.Disabled !in fetchSemanticsNode().config

    protected fun layoutOf(node: SemanticsNode): TextLayoutResult? {
        val action = node.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action ?: return null
        val results = mutableListOf<TextLayoutResult>()
        return if (action.invoke(results)) results.firstOrNull() else null
    }

    /** 这条消息的正文文字节点，跳过引用块和脚注。 */
    protected fun bodyNodes(index: Int): List<SemanticsNode> = compose.onAllNodes(
        hasAnyAncestor(hasTestTag("item-ui-fake-$index")) and
            SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult) and
            !hasAnyAncestor(hasTestTag("quote")) and
            !hasTestTag("messageTime"),
        useUnmergedTree = true,
    ).fetchSemanticsNodes()

    /** 节点的每个字行占的框，`reserve` 个字符是末尾的透明占位，不算正文。 */
    protected fun lineBoxes(node: SemanticsNode, reserve: Int): List<Rect> {
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
    protected fun Rect.expandTo(other: Rect) = Rect(
        minOf(left, other.left),
        minOf(top, other.top),
        maxOf(right, other.right),
        maxOf(bottom, other.bottom),
    )

    /** 选项的选中和停用语义在内层那行，标签在外层，按祖先找；选中与否两种状态都有 `Selected`。 */
    protected fun choice(key: String) = compose.onNode(
        hasAnyAncestor(hasTestTag("choice-$key")) and
            SemanticsMatcher.keyIsDefined(SemanticsProperties.Selected),
        useUnmergedTree = true,
    )

    protected fun string(id: Int) = compose.activity.getString(id)

    /** 状态栏和键盘的高度（像素），从窗口的实时 insets 取。 */
    protected fun statusBarHeight(): Int =
        compose.activity.window.decorView.rootWindowInsets.getInsets(WindowInsets.Type.statusBars()).top

    protected fun imeHeight(): Int =
        compose.activity.window.decorView.rootWindowInsets.getInsets(WindowInsets.Type.ime()).bottom

    /** 键盘动画比布局慢一拍，等表面画上来再截，不然截到的还是刚才那一帧。 */
    protected fun settleKeyboard() {
        compose.waitUntil(10_000) { imeHeight() > 0 }
        compose.waitForIdle()
        Thread.sleep(300)
    }

    protected fun pillVisible() = compose.onAllNodes(
        hasTestTag("date-pill"),
        useUnmergedTree = true,
    ).fetchSemanticsNodes().isNotEmpty()

    protected fun screenshot(name: String) {
        // 滚过以后浮动日期胶囊还在，等它淡出再拍，不然截到的是它盖在字上的样子；失败现场可能已经停了，等不到也照拍。
        runCatching { compose.waitUntil(5_000) { !pillVisible() } }
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "telegram")
        check(directory.isDirectory || directory.mkdirs())
        val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        FileOutputStream(File(directory, "$name.png")).use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }

    /** 回到房间列表再进群；用例都在同一个活动里连着跑。 */
    protected fun openFamily() {
        compose.onNodeWithContentDescription(string(R.string.design_back)).performClick()
        compose.onNodeWithText("Family").performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().size == 1 }
    }

    protected fun avatars() =
        compose.onAllNodes(hasTestTag("sender-avatar"), useUnmergedTree = true).fetchSemanticsNodes()

    /** 气泡所在消息的序号：按外层条目的标签认，引用的文字里也会出现“Fixture message N”。 */
    protected fun fixtureIndexOf(bubble: SemanticsNode): Int? {
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
    protected fun texts(node: SemanticsNode): String =
        node.config.getOrNull(SemanticsProperties.Text).orEmpty().joinToString() +
            node.children.joinToString("") { texts(it) }

    /** 假数据第 `index` 条的日期：第 0 条早两天，11 起是后一天，其余同一天，每条隔一分钟（同 `ui_fake_room.rs`）。 */
    protected fun fixtureDate(index: Int): String {
        val at = 1_792_000_000_000L + index * 60_000L +
            if (index >= 11) 86_400_000L else 0L -
            if (index == 0) 172_800_000L else 0L
        return DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(at))
    }

    /** 面板发送条上的发送键：带了那个面板的标签，和房间里被面板挡住的发送键区分开。 */
    protected fun sheetSend(tag: String) =
        compose.onNode(hasTestTag(tag) and hasContentDescription(string(R.string.composer_send)))

    protected fun hasBack() =
        compose.onAllNodes(hasContentDescription(string(R.string.design_back))).fetchSemanticsNodes().isNotEmpty()

    protected fun forwardRows() = compose.onAllNodes(hasTestTag("forward-row"))

    /**
     * 房间打开了（顶栏是它的名字），并且时间线里有正文含 `text` 的条目。房间刚开时落在老消息中间，点“向下”回最新。
     * 转发到的房间是现开的，副本的编号取决于这个房间之前发过多少条，按正文认比按编号稳。
     */
    protected fun forwardedInto(room: String, text: String): Boolean {
        val bar = compose.onAllNodes(
            hasText(room) and hasAnyAncestor(hasTestTag("room-bar")),
            useUnmergedTree = true,
        ).fetchSemanticsNodes()
        val open = bar.isNotEmpty()
        if (open && itemsWithText(text).isEmpty()) {
            // 名字后面可能还带着新消息数，按前缀认。
            runCatching {
                compose.onNodeWithContentDescription(string(R.string.room_scroll_to_bottom), substring = true)
                    .performClick()
            }
        }
        return open && itemsWithText(text).isNotEmpty()
    }

    /** 时间线里正文含 `needle` 的条目。 */
    protected fun itemsWithText(needle: String): List<SemanticsNode> = compose.onAllNodes(
        SemanticsMatcher("含「$needle」的条目") { node ->
            node.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("item-") == true &&
                texts(node).contains(needle)
        },
        useUnmergedTree = true,
    ).fetchSemanticsNodes()

    /**
     * 把附件区里留着的附件一个个拿掉：一排照片会横着滚，先把行滚回开头再点第一个拿掉的按钮，
     * 否则点到的是滚出屏幕的格子。
     */
    protected fun clearTray() {
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

    protected fun trayTiles(): Int = compose.onAllNodes(
        SemanticsMatcher("附件区里的照片") { node ->
            node.config.getOrNull(SemanticsProperties.ContentDescription).orEmpty()
                .any { it == "photo.png" || it == "photo.jpg" }
        },
        useUnmergedTree = true,
    ).fetchSemanticsNodes().size

    /**
     * 相册开头放两张一定能读的照片（`pick-` 开头），返回它们的 URI；先删上一轮留下的照片。
     * 名字带时间戳：删掉的行，文件可能还在盘上，同名插进去 MediaProvider 要改名排队，偶尔 "Failed to build unique file"。
     * 插入时给的 DATE_ADDED 会被 MediaProvider 冲成插入那一秒，同一秒里的先后按 _id 排——
     * 所以别假定开头的两张就是它们，用 [albumIndexOf] 查网格里的下标。
     */
    protected fun ensureAlbumPhotos(): List<Uri> {
        val resolver = instrumentation.targetContext.contentResolver
        val collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        runCatching {
            resolver.delete(
                collection,
                "${MediaStore.Images.Media.DISPLAY_NAME} LIKE 'pick-%'" +
                    " OR ${MediaStore.Images.Media.DISPLAY_NAME} LIKE 'broken%'",
                null,
            )
        }
        val added = mutableListOf<Uri>()
        val stamp = System.currentTimeMillis()
        repeat(2) { index ->
            val values = ContentValues().apply {
                put(MediaStore.Images.Media.DISPLAY_NAME, "pick-$stamp-$index.png")
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
            added += uri
        }
        return added
    }

    /** 照片在相册网格里的下标：和面板一样按 `DATE_ADDED DESC` 查一遍（同一秒里的先后按 _id）。 */
    protected fun albumIndexOf(uri: Uri): Int {
        val resolver = instrumentation.targetContext.contentResolver
        val id = ContentUris.parseId(uri)
        val index = resolver.query(
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            arrayOf(MediaStore.Images.Media._ID),
            null,
            null,
            "${MediaStore.Images.Media.DATE_ADDED} DESC",
        )?.use { cursor ->
            var position = 0
            while (cursor.moveToNext()) {
                if (cursor.getLong(0) == id) return@use position
                position++
            }
            -1
        } ?: -1
        check(index >= 0) { "照片不在相册里" }
        return index
    }

    protected fun photos() = compose.onAllNodes(hasTestTag("album-photo"))

    /** 打开回形针面板，等到相册网格出来。 */
    protected fun openAlbum() {
        compose.onNodeWithContentDescription(string(R.string.composer_attach)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasTestTag("album-photo")).fetchSemanticsNodes().isNotEmpty() }
    }
}
