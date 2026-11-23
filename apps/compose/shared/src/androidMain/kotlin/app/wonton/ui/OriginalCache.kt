package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.res.stringResource
import app.wonton.client.ClientCore
import app.wonton.protocol.MediaHandle

/** 组里每张原图的下载结果：按 media 留着，翻走再翻回来不重下，相邻页的预取也不白做。 */
internal class OriginalCache {
    val loaded = mutableStateMapOf<MediaHandle, Result<ImageBitmap>?>()
    val attempts = mutableStateMapOf<MediaHandle, Int>()

    /** 这张重新取一次：先丢掉结果，让 [rememberOriginal] 的守卫放行，再换重试键。 */
    fun retry(media: MediaHandle) {
        loaded[media] = null
        attempts[media] = attempts.getOrDefault(media, 0) + 1
    }
}

/** 取一张原图；[cache] 里已经有结果就不再取，重试次数变了才重新取一次。 */
@Composable
internal fun rememberOriginal(
    core: ClientCore,
    media: MediaHandle,
    cache: OriginalCache,
): Result<ImageBitmap>? {
    val attempt = cache.attempts.getOrDefault(media, 0)
    val formatUnsupported = stringResource(R.string.message_image_format_unsupported)
    LaunchedEffect(media, attempt) {
        if (cache.loaded[media] != null) return@LaunchedEffect
        cache.loaded[media] = null
        cache.loaded[media] = decodeOriginalMedia(core, media, formatUnsupported)
    }
    return cache.loaded[media]
}
