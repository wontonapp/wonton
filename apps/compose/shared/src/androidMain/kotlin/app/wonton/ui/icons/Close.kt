package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Close: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Close",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 205.66 194.34 a 8.0 8.0 0.0 0 1 -11.32 11.32 L 128.0 139.31 L 61.66 205.66 a 8.0 8.0 0.0 0 1 -11.32 -11.32 L 116.69 128.0 L 50.34 61.66 A 8.0 8.0 0.0 0 1 61.66 50.34 L 128.0 116.69 l 66.34 -66.35 a 8.0 8.0 0.0 0 1 11.32 11.32 L 139.31 128.0 Z")
        )
    }.build()
}
