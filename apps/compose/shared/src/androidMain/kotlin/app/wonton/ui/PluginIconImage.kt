package app.wonton.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wonton.protocol.PluginIcon
import app.wonton.protocol.PluginImage

/** 和 iOS 的应用图标同样的圆角比例。 */
private const val CORNER_RATIO = 0.2237f
private const val GLYPH_RATIO = 0.42f

/** 列表和主屏用小图，详情和确认页用大图。 */
private val LARGE_FROM = 96.dp

private val placeholder = Brush.verticalGradient(listOf(Color(0xFFC7C7CC), Color(0xFF8E8E93)))

/** 插件图标，统一裁成圆角方形（ADR-0017：图标满幅，不自己画圆角）。没有图标、取不到时画名字的第一个字做占位。 */
@Composable
fun PluginIconImage(icon: PluginIcon?, title: String, size: Dp, plugins: PluginsState) {
    val shape = pluginIconShape(size)
    val image = rememberPluginImage(plugins, if (size > LARGE_FROM) icon?.large else icon?.small)
    Box(
        Modifier.size(size).clip(shape).border(0.5.dp, Color.Black.copy(alpha = 0.08f), shape),
        contentAlignment = Alignment.Center,
    ) {
        if (image != null) {
            Image(image, contentDescription = null, Modifier.size(size), contentScale = ContentScale.Crop)
        } else {
            val glyph = (size.value * GLYPH_RATIO).sp
            Box(Modifier.size(size).background(placeholder))
            Text(title.take(1).uppercase(), color = Color.White, fontSize = glyph, fontWeight = FontWeight.Medium)
        }
    }
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
