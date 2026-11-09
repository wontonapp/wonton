@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package app.wonton.ui

import android.graphics.ImageDecoder
import android.graphics.drawable.AnimatedImageDrawable
import android.net.Uri
import android.view.TextureView
import android.widget.ImageView
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import app.wonton.client.ClientCore
import app.wonton.protocol.AvatarImage
import app.wonton.protocol.CoreException
import app.wonton.protocol.MediaHandle
import app.wonton.protocol.PluginIcon
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** 静态头像：字节经核心的 `loadMedia` 取，解码走 [AvatarStillCache]；解不出来落到静态兜底或应用图标。 */
@Composable
internal fun StillAvatar(
    image: AvatarImage,
    plugins: PluginsState,
    size: Dp,
    fallbackStill: AvatarImage?,
    fallbackIcon: PluginIcon?,
) {
    val state by produceState<Result<ImageBitmap>?>(null, image.media) {
        value = AvatarStillCache.load(plugins.core, image.media)
    }
    AvatarDecodedImage(state, size) { AvatarFallback(fallbackStill, fallbackIcon, plugins, size) }
}

/** 动图（GIF、动图 WebP）：平台解码器播；不该播时显示首帧（没有就静态兜底）。 */
@Composable
internal fun AnimatedAvatar(
    image: AvatarImage,
    allowed: Boolean,
    plugins: PluginsState,
    size: Dp,
    fallbackStill: AvatarImage?,
    fallbackIcon: PluginIcon?,
) {
    val playing = rememberPlayingSlot(allowed)
    var drawable by remember(image.media) { mutableStateOf<AnimatedImageDrawable?>(null) }
    LaunchedEffect(image.media, playing) {
        if (!playing || drawable != null) return@LaunchedEffect
        drawable = decodeAnimated(plugins.core, image.media)
    }
    val current = drawable
    if (playing && current != null) {
        AndroidView(
            factory = { context ->
                ImageView(context).apply {
                    scaleType = ImageView.ScaleType.CENTER_CROP
                    setImageDrawable(current)
                }
            },
            update = {
                it.setImageDrawable(current)
                // 名额、前台和省电状态变了就停播；停在哪一帧就显示哪一帧。
                if (playing) current.start() else current.stop()
            },
            onRelease = { current.stop() },
            modifier = Modifier.size(size).clip(CircleShape),
        )
    } else {
        PosterOrFallback(image.poster, fallbackStill, fallbackIcon, plugins, size)
    }
}

/** 解码不出动图（格式坏、取不到）时为 null，落到首帧或静态兜底。 */
@Suppress("SwallowedException")
private suspend fun decodeAnimated(core: ClientCore, media: MediaHandle): AnimatedImageDrawable? =
    try {
        val bytes = core.loadMedia(media)
        ImageDecoder.decodeDrawable(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) as? AnimatedImageDrawable
    } catch (failure: CoreException) {
        null
    } catch (failure: IOException) {
        null
    }

/** 视频（MP4）：核心给本机缓存文件，Media3 静音循环播；首帧出来之前和不该播时显示首帧。 */
@Suppress("SwallowedException")
@Composable
internal fun VideoAvatar(
    image: AvatarImage,
    allowed: Boolean,
    plugins: PluginsState,
    size: Dp,
    fallbackStill: AvatarImage?,
    fallbackIcon: PluginIcon?,
) {
    val playing = rememberPlayingSlot(allowed)
    // 用到哪个状态才下载哪个；下完之前显示首帧（规格第 1 节）。
    var file by remember(image.media) { mutableStateOf<String?>(null) }
    LaunchedEffect(image.media, playing) {
        if (!playing || file != null) return@LaunchedEffect
        file = try {
            plugins.core.avatarFile(image.media)
        } catch (failure: CoreException) {
            null
        }
    }
    Box(Modifier.size(size)) {
        PosterOrFallback(image.poster, fallbackStill, fallbackIcon, plugins, size)
        val path = file
        if (playing && path != null) VideoSurface(path)
    }
}

@Composable
private fun VideoSurface(path: String) {
    val context = LocalContext.current
    val player = remember(path) {
        ExoPlayer.Builder(context).build().apply {
            setMediaItem(MediaItem.fromUri(Uri.fromFile(File(path))))
            setRepeatMode(Player.REPEAT_MODE_ONE)
            // 静音、不请求音频焦点：头像不出声，也不打断主人在放的别的声音。
            volume = 0f
            videoScalingMode = C.VIDEO_SCALING_MODE_SCALE_TO_FIT_WITH_CROPPING
            prepare()
            playWhenReady = true
        }
    }
    DisposableEffect(player) {
        onDispose { player.release() }
    }
    AndroidView(
        factory = { TextureView(it) },
        update = { player.setVideoTextureView(it) },
        modifier = Modifier.fillMaxSize().clip(CircleShape),
    )
}

/** 首帧或静态兜底：动图和视频不该播时的样子。 */
@Composable
private fun PosterOrFallback(
    poster: MediaHandle?,
    fallbackStill: AvatarImage?,
    fallbackIcon: PluginIcon?,
    plugins: PluginsState,
    size: Dp,
) {
    when {
        poster != null -> PosterImage(poster, plugins, size, fallbackStill, fallbackIcon)
        fallbackStill != null -> PosterImage(fallbackStill.media, plugins, size, null, fallbackIcon)
        else -> PluginIconImage(fallbackIcon, size, plugins)
    }
}

/** 首帧也是一张静态图：同样走解码缓存，解不出来落到静态头像或图标。 */
@Composable
private fun PosterImage(
    media: MediaHandle,
    plugins: PluginsState,
    size: Dp,
    fallbackStill: AvatarImage?,
    fallbackIcon: PluginIcon?,
) {
    val state by produceState<Result<ImageBitmap>?>(null, media) {
        value = AvatarStillCache.load(plugins.core, media)
    }
    AvatarDecodedImage(state, size) { AvatarFallback(fallbackStill, fallbackIcon, plugins, size) }
}

@Composable
private fun AvatarDecodedImage(state: Result<ImageBitmap>?, size: Dp, onFailed: @Composable () -> Unit) {
    when {
        state == null -> Box(Modifier.size(size).background(WontonTheme.colors.surfaceSubtle.toColor(), CircleShape))
        state.isSuccess -> CircularImage(state.getOrThrow(), size)
        else -> onFailed()
    }
}

@Composable
private fun CircularImage(bitmap: ImageBitmap, size: Dp) {
    Image(
        bitmap,
        contentDescription = null,
        modifier = Modifier.size(size).clip(CircleShape),
        contentScale = ContentScale.Crop,
    )
}

/** 状态头像没有时的静态头像，再没有用应用的图标（就是列表行的做法）。 */
@Composable
private fun AvatarFallback(still: AvatarImage?, icon: PluginIcon?, plugins: PluginsState, size: Dp) {
    if (still == null) {
        PluginIconImage(icon, size, plugins)
    } else {
        val state by produceState<Result<ImageBitmap>?>(null, still.media) {
            value = AvatarStillCache.load(plugins.core, still.media)
        }
        AvatarDecodedImage(state, size) { PluginIconImage(icon, size, plugins) }
    }
}
