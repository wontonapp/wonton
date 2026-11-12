package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.ErrorFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "ErrorFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 128.0 24.0 A 104.0 104.0 0.0 1 0 232.0 128.0 A 104.11 104.11 0.0 0 0 128.0 24.0 Z M 165.66 154.34 a 8.0 8.0 0.0 0 1 -11.32 11.32 L 128.0 139.31 l -26.34 26.35 a 8.0 8.0 0.0 0 1 -11.32 -11.32 L 116.69 128.0 L 90.34 101.66 a 8.0 8.0 0.0 0 1 11.32 -11.32 L 128.0 116.69 l 26.34 -26.35 a 8.0 8.0 0.0 0 1 11.32 11.32 L 139.31 128.0 Z")
        )
    }.build()
}
