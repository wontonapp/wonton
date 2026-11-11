package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Error: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Error",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 165.66 101.66 L 139.31 128.0 l 26.35 26.34 a 8.0 8.0 0.0 0 1 -11.32 11.32 L 128.0 139.31 l -26.34 26.35 a 8.0 8.0 0.0 0 1 -11.32 -11.32 L 116.69 128.0 L 90.34 101.66 a 8.0 8.0 0.0 0 1 11.32 -11.32 L 128.0 116.69 l 26.34 -26.35 a 8.0 8.0 0.0 0 1 11.32 11.32 Z M 232.0 128.0 A 104.0 104.0 0.0 1 1 128.0 24.0 A 104.11 104.11 0.0 0 1 232.0 128.0 Z M 216.0 128.0 a 88.0 88.0 0.0 1 0 -88.0 88.0 A 88.1 88.1 0.0 0 0 216.0 128.0 Z")
        )
    }.build()
}
