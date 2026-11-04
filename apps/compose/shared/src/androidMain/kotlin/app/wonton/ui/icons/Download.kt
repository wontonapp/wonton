package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Download: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Download",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 224.0 144.0 v 64.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 40.0 216.0 a 8.0 8.0 0.0 0 1 -8.0 -8.0 L 32.0 144.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 56.0 L 208.0 200.0 L 208.0 144.0 a 8.0 8.0 0.0 0 1 16.0 0.0 Z M 122.34 149.66 a 8.0 8.0 0.0 0 0 11.32 0.0 l 40.0 -40.0 a 8.0 8.0 0.0 0 0 -11.32 -11.32 L 136.0 124.69 L 136.0 32.0 a 8.0 8.0 0.0 0 0 -16.0 0.0 v 92.69 L 93.66 98.34 a 8.0 8.0 0.0 0 0 -11.32 11.32 Z")
        )
    }.build()
}
