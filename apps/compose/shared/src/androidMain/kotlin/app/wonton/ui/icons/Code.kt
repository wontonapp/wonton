package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Code: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Code",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 69.12 94.15 L 28.5 128.0 l 40.62 33.85 a 8.0 8.0 0.0 1 1 -10.24 12.29 l -48.0 -40.0 a 8.0 8.0 0.0 0 1 0.0 -12.29 l 48.0 -40.0 a 8.0 8.0 0.0 0 1 10.24 12.3 Z M 245.12 121.85 L 197.12 81.85 a 8.0 8.0 0.0 1 0 -10.24 12.3 L 227.5 128.0 l -40.62 33.85 a 8.0 8.0 0.0 1 0 10.24 12.29 l 48.0 -40.0 a 8.0 8.0 0.0 0 0 0.0 -12.29 Z M 162.73 32.48 a 8.0 8.0 0.0 0 0 -10.25 4.79 l -64.0 176.0 a 8.0 8.0 0.0 0 0 4.79 10.26 A 8.14 8.14 0.0 0 0 96.0 224.0 a 8.0 8.0 0.0 0 0 7.52 -5.27 l 64.0 -176.0 A 8.0 8.0 0.0 0 0 162.73 32.48 Z")
        )
    }.build()
}
