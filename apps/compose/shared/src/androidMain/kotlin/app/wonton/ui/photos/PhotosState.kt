package app.wonton.ui.photos

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.PhotoEdit
import app.wonton.protocol.PhotoEditKind
import app.wonton.protocol.PhotoTile
import app.wonton.protocol.PhotosAction
import app.wonton.protocol.PhotosViewModel
import app.wonton.ui.Problem
import app.wonton.ui.send
import java.util.TimeZone

private const val MINUTE_MS = 60_000

/** 本机用户写成 Matrix ID（`@mom:example.com`）时只显示 `@` 和 `:` 之间那一段；本来就是用户名的原样。 */
internal fun String.matrixLocalPart(): String =
    if (startsWith("@")) substring(1).substringBefore(':') else this

/**
 * 相册页和核心之间的桥：持有核心推来的 [PhotosViewModel]，把操作发回去。游标、按日分组和选择集都在核心。
 */
internal class PhotosState(val core: ClientCore, val account: String?, private val pluginId: String) {
    var view by mutableStateOf(core.photos())
        private set

    /** 核心拒收操作的原因，弹窗显示。 */
    var rejection by mutableStateOf<Problem?>(null)

    /** 用的是哪个相册应用（插件 id）：用户挑过的那家，否则是点开的这一个。图片缓存的键带着它。 */
    val source: String get() = view.sources.chosen ?: pluginId

    suspend fun observe() {
        core.subscribePhotos().use { subscription ->
            while (true) view = subscription.next() ?: break
        }
    }

    fun send(action: PhotosAction) {
        core.send(Action.Photos(action))?.let { rejection = it }
    }

    /** 进相册页：点开的是哪个相册应用就点名用它。时差按手机此刻的算，核心用它按日分组。 */
    fun open() {
        val offset = TimeZone.getDefault().getOffset(System.currentTimeMillis()) / MINUTE_MS
        send(PhotosAction.Open(offset, pluginId))
    }

    fun edit(edit: PhotoEdit) = send(PhotosAction.Edit(edit))

    /** 这个相册应用接不接这种写操作；还没取到时都当不接，不出按钮。 */
    fun supports(kind: PhotoEditKind): Boolean = view.actions?.contains(kind) == true

    /** 缩略图、大图的引用：缓存按它的键存，不按回环地址。 */
    fun ref(id: String, rendition: app.wonton.protocol.PhotoRendition) =
        PhotoRef(account, source, id, rendition)

    fun personRef(id: String) = PhotoRef(account, source, id, null)
}

/** 时间线上已经取到的格子，按显示的先后。 */
internal fun PhotosViewModel.timelineTiles(): List<PhotoTile> =
    timeline.months.flatMap { month -> month.days.flatMap { it.tiles } }
