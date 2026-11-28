package app.wonton.ui.music

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import app.wonton.client.ClientCore
import app.wonton.protocol.MusicAction
import app.wonton.protocol.MusicFailure
import app.wonton.protocol.MusicPage
import app.wonton.protocol.MusicPageAction
import app.wonton.protocol.MusicProblem
import app.wonton.protocol.MusicTarget
import app.wonton.protocol.PlaylistRow
import app.wonton.ui.Problem
import app.wonton.ui.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.Locale

/** 封面的内存缓存上限（字节）；盘上的缓存在核心。 */
private const val COVER_CACHE_BYTES = 32 * 1024 * 1024

/** 向核心要的封面边长按这个粒度取整：同一张封面在宽度差几像素的格子里命中同一份。 */
private const val COVER_PIXEL_STEP = 64

private const val MS_PER_SECOND = 1000uL
private const val MS_PER_MINUTE = 60_000uL
private const val SECONDS_PER_HOUR = 3600uL
private const val SECONDS_PER_MINUTE = 60uL

/** 一页音乐内容：订阅核心的那一页，壳只画 [view]。 */
internal class MusicPageModel(core: ClientCore, pluginId: String, val page: MusicPage, scope: CoroutineScope) {
    private val subscription = core.subscribeMusicPage(pluginId, page)
    var view by mutableStateOf(subscription.view())
        private set

    init {
        // cancel() 让 next() 回空，循环结束时 use 释放订阅；页面整个离开时协程被取消，Rust 侧的 Drop 也会取消。
        scope.launch { subscription.use { while (true) view = it.next() ?: break } }
    }

    fun act(action: MusicPageAction) = subscription.act(action)

    fun close() = subscription.cancel()
}

/** 栈里的一页；[key] 认页（同一页面可以推两次），[depth] 定转场方向。 */
internal class MusicEntry(val key: Int, val depth: Int, val model: MusicPageModel) {
    val page: MusicPage get() = model.page
}

/**
 * 音乐库里推进去的页：只在壳里，跟着打开它的插件页走（核心的页面栈只有这一个插件页）。
 * 每页的订阅跟着它在栈里活：盖住时不重订（重订就是重取一遍），弹出时取消。
 */
internal class MusicNavigator(val core: ClientCore, val pluginId: String, private val scope: CoroutineScope) {
    private var keys = 0
    val entries = mutableStateListOf(entry(MusicPage.Home))

    /** 收藏没成功：弹一句。 */
    var actionProblem by mutableStateOf<Problem?>(null)

    /** 弹出的页留下的界面状态（滚动位置）由持有者清掉。 */
    var forget: (Int) -> Unit = {}

    val top: MusicEntry get() = entries.last()
    val canGoBack: Boolean get() = entries.size > 1

    fun open(page: MusicPage) {
        entries.add(entry(page))
    }

    /** 换掉最上面这一页（专辑换排序）：不多一层返回。 */
    fun replace(page: MusicPage) {
        drop(entries.removeAt(entries.lastIndex))
        entries.add(entry(page))
    }

    fun back() {
        if (canGoBack) drop(entries.removeAt(entries.lastIndex))
    }

    fun close() = entries.forEach { it.model.close() }

    fun star(target: MusicTarget, starred: Boolean) {
        scope.launch {
            core.musicAct(pluginId, MusicAction.Star(target, starred))?.let { actionProblem = it.problem }
        }
    }

    suspend fun cover(cover: String, pixels: Int): Bitmap? = MusicCovers.load(core, pluginId, cover, pixels)

    private fun drop(entry: MusicEntry) {
        entry.model.close()
        forget(entry.key)
    }

    private fun entry(page: MusicPage): MusicEntry {
        val depth = if (keys == 0) 0 else entries.size
        return MusicEntry(keys++, depth, MusicPageModel(core, pluginId, page, scope))
    }
}

internal val LocalMusic = compositionLocalOf<MusicNavigator> { error("MusicNavigator is not provided") }

/** 封面图：按插件、封面、边长缓存在内存里。 */
internal object MusicCovers {
    private val images = object : LruCache<String, Bitmap>(COVER_CACHE_BYTES) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount
    }

    fun pixels(width: Int): Int = (width + COVER_PIXEL_STEP - 1) / COVER_PIXEL_STEP * COVER_PIXEL_STEP

    fun cached(pluginId: String, cover: String, pixels: Int): Bitmap? = images.get(key(pluginId, cover, pixels))

    suspend fun load(core: ClientCore, pluginId: String, cover: String, pixels: Int): Bitmap? {
        val key = key(pluginId, cover, pixels)
        return images.get(key) ?: core.musicCover(pluginId, cover, pixels.toUInt())
            ?.let { bytes -> withContext(Dispatchers.Default) { BitmapFactory.decodeByteArray(bytes, 0, bytes.size) } }
            ?.also { images.put(key, it) }
    }

    private fun key(pluginId: String, cover: String, pixels: Int) = "$pluginId/$cover/$pixels"
}

/** 类别由核心给，文字按类别写在这里；原样的原因收进详情。 */
internal val MusicProblem.problem: Problem
    get() = when (val failure = failure) {
        MusicFailure.Transport -> Problem(
            titleRes = R.string.problem_transport_unavailable_title,
            nextStepRes = R.string.problem_transport_unavailable_next_step, detail = details,
        )
        MusicFailure.PluginNotReady -> Problem(
            titleRes = R.string.music_problem_not_ready_title,
            nextStepRes = R.string.music_problem_not_ready_next_step, detail = details,
        )
        MusicFailure.NotSignedIn -> Problem(
            titleRes = R.string.music_problem_not_signed_in_title,
            nextStepRes = R.string.music_problem_not_signed_in_next_step, detail = details,
        )
        MusicFailure.PermissionDenied -> Problem(
            titleRes = R.string.music_problem_forbidden_title,
            nextStepRes = R.string.music_problem_forbidden_next_step, detail = details,
        )
        MusicFailure.NotFound -> Problem(
            titleRes = R.string.music_problem_not_found_title,
            nextStepRes = R.string.music_problem_not_found_next_step, detail = details,
        )
        MusicFailure.Upstream -> Problem(
            titleRes = R.string.music_problem_upstream_title,
            nextStepRes = R.string.problem_upstream_next_step, detail = details,
        )
        MusicFailure.Protocol -> Problem(
            titleRes = R.string.music_problem_protocol_title,
            nextStepRes = R.string.problem_plugin_protocol_next_step, detail = details,
        )
        is MusicFailure.Other -> Problem(
            titleRes = R.string.music_problem_unknown_title,
            nextStepRes = R.string.problem_upstream_next_step, detail = details ?: failure.v1,
        )
    }

/** 翻页：滚到最后几项时要下一页。 */
internal class MusicPaging(val hasMore: Boolean, val loadingMore: Boolean, private val loadMore: () -> Unit) {
    fun reached() {
        if (hasMore && !loadingMore) loadMore()
    }
}

internal object MusicFormat {
    /** 歌的时长：`3:07`，一小时以上 `1:02:07`。 */
    fun clock(milliseconds: ULong): String {
        val seconds = milliseconds / MS_PER_SECOND
        val hours = seconds / SECONDS_PER_HOUR
        val minutes = seconds / SECONDS_PER_MINUTE % SECONDS_PER_MINUTE
        val rest = seconds % SECONDS_PER_MINUTE
        return if (hours > 0uL) {
            String.format(Locale.ROOT, "%d:%02d:%02d", hours.toLong(), minutes.toLong(), rest.toLong())
        } else {
            String.format(Locale.ROOT, "%d:%02d", minutes.toLong(), rest.toLong())
        }
    }

    /** 专辑和歌单底下的一行：几首、多少分钟。 */
    @Composable
    fun summary(songs: UInt, milliseconds: ULong): String {
        val minutes = ((milliseconds + MS_PER_MINUTE - 1uL) / MS_PER_MINUTE).toInt()
        val count = pluralStringResource(R.plurals.music_song_count, songs.toInt(), songs.toInt())
        val length = pluralStringResource(R.plurals.music_minutes, minutes, minutes)
        return stringResource(R.string.music_summary, count, length)
    }

    @Composable
    fun playlistLine(playlist: PlaylistRow): String {
        val count = pluralStringResource(
            R.plurals.music_song_count, playlist.songCount.toInt(), playlist.songCount.toInt(),
        )
        return playlist.owner?.let { "$it · $count" } ?: count
    }
}
