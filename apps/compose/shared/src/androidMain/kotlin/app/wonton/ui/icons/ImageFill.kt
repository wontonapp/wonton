package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.ImageFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "ImageFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 40.0 L 40.0 40.0 A 16.0 16.0 0.0 0 0 24.0 56.0 L 24.0 200.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 216.0 216.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 232.0 56.0 A 16.0 16.0 0.0 0 0 216.0 40.0 Z M 156.0 88.0 a 12.0 12.0 0.0 1 1 -12.0 12.0 A 12.0 12.0 0.0 0 1 156.0 88.0 Z M 216.0 200.0 L 40.0 200.0 L 40.0 160.69 l 46.34 -46.35 a 8.0 8.0 0.0 0 1 11.32 0.0 h 0.0 L 165.0 181.66 a 8.0 8.0 0.0 0 0 11.32 -11.32 l -17.66 -17.65 L 173.0 138.34 a 8.0 8.0 0.0 0 1 11.31 0.0 L 216.0 170.07 L 216.0 200.0 Z")
        )
    }.build()
}
