package app.wonton.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.wonton.protocol.PluginIcon
import app.wonton.protocol.PluginImage
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.accentFill
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.AppsFill
import app.wonton.ui.icons.WontonIcon

/** 和 iOS 的应用图标同样的圆角比例。 */
private const val CORNER_RATIO = 0.2237f
private const val GLYPH_RATIO = 0.54f

/** 列表和主屏用小图，详情和确认页用大图。 */
private val LARGE_FROM = 96.dp

/**
 * 插件图标，统一裁成圆角方形（ADR-0017：图标满幅，不自己画圆角）。没有图标、取不到时画同一个占位（ADR-0026）。
 * 还在取的时候只画底色：路径慢时要等一会儿，画占位看着就像这个插件没有图标。
 */
@Composable
fun PluginIconImage(icon: PluginIcon?, size: Dp, plugins: PluginsState) {
    val shape = pluginIconShape(size)
    val source = if (size > LARGE_FROM) icon?.large else icon?.small
    val state by produceState<IconState>(IconState.Loading, source) {
        value = source?.let { plugins.imageOrNull(it) }?.let { IconState.Loaded(it) } ?: IconState.Missing
    }
    val colors = WontonTheme.colors
    when (val current = state) {
        is IconState.Loaded -> Image(
            current.image,
            contentDescription = null,
            Modifier.size(size).clip(shape).border(WontonTheme.size.hairline, colors.separator.toColor(), shape),
            contentScale = ContentScale.Crop,
        )
        IconState.Loading -> Box(Modifier.size(size).clip(shape).background(colors.surfaceSubtle.toColor()))
        IconState.Missing -> Box(Modifier.size(size).accentFill(shape), contentAlignment = Alignment.Center) {
            Icon(
                WontonIcon.AppsFill,
                contentDescription = null,
                modifier = Modifier.size(size * GLYPH_RATIO),
                tint = colors.accentInkOnFill.toColor(),
            )
        }
    }
}

private sealed interface IconState {
    data object Loading : IconState
    data object Missing : IconState
    data class Loaded(val image: ImageBitmap) : IconState
}

fun pluginIconShape(size: Dp) = RoundedCornerShape(size * CORNER_RATIO)

/** 取插件的图片；取到之前和取不到时为 null。 */
@Composable
fun rememberPluginImage(plugins: PluginsState, source: PluginImage?): ImageBitmap? {
    val image by produceState<ImageBitmap?>(null, source) {
        value = source?.let { plugins.imageOrNull(it) }
    }
    return image
}
