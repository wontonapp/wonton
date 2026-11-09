package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.TasksFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "TasksFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 208.0 32.0 L 48.0 32.0 A 16.0 16.0 0.0 0 0 32.0 48.0 L 32.0 208.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 208.0 224.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 224.0 48.0 A 16.0 16.0 0.0 0 0 208.0 32.0 Z M 117.66 149.66 l -32.0 32.0 a 8.0 8.0 0.0 0 1 -11.32 0.0 l -16.0 -16.0 a 8.0 8.0 0.0 0 1 11.32 -11.32 L 80.0 164.69 l 26.34 -26.35 a 8.0 8.0 0.0 0 1 11.32 11.32 Z M 117.66 85.66 L 85.66 117.66 a 8.0 8.0 0.0 0 1 -11.32 0.0 l -16.0 -16.0 A 8.0 8.0 0.0 0 1 69.66 90.34 L 80.0 100.69 l 26.34 -26.35 a 8.0 8.0 0.0 0 1 11.32 11.32 Z M 192.0 168.0 L 144.0 168.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 48.0 a 8.0 8.0 0.0 0 1 0.0 16.0 Z M 192.0 104.0 L 144.0 104.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 48.0 a 8.0 8.0 0.0 0 1 0.0 16.0 Z")
        )
    }.build()
}
