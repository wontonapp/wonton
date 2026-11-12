package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Send: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Send",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 205.66 117.66 a 8.0 8.0 0.0 0 1 -11.32 0.0 L 136.0 59.31 V 216.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 V 59.31 L 61.66 117.66 a 8.0 8.0 0.0 0 1 -11.32 -11.32 l 72.0 -72.0 a 8.0 8.0 0.0 0 1 11.32 0.0 l 72.0 72.0 A 8.0 8.0 0.0 0 1 205.66 117.66 Z")
        )
    }.build()
}
