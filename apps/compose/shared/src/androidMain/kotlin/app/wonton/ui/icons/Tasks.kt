package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Tasks: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Tasks",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 224.0 128.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 128.0 136.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 88.0 A 8.0 8.0 0.0 0 1 224.0 128.0 Z M 128.0 72.0 h 88.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 L 128.0 56.0 a 8.0 8.0 0.0 0 0 0.0 16.0 Z M 216.0 184.0 L 128.0 184.0 a 8.0 8.0 0.0 0 0 0.0 16.0 h 88.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 Z M 82.34 42.34 L 56.0 68.69 L 45.66 58.34 A 8.0 8.0 0.0 0 0 34.34 69.66 l 16.0 16.0 a 8.0 8.0 0.0 0 0 11.32 0.0 l 32.0 -32.0 A 8.0 8.0 0.0 0 0 82.34 42.34 Z M 82.34 106.34 L 56.0 132.69 L 45.66 122.34 a 8.0 8.0 0.0 0 0 -11.32 11.32 l 16.0 16.0 a 8.0 8.0 0.0 0 0 11.32 0.0 l 32.0 -32.0 a 8.0 8.0 0.0 0 0 -11.32 -11.32 Z M 82.34 170.34 L 56.0 196.69 L 45.66 186.34 a 8.0 8.0 0.0 0 0 -11.32 11.32 l 16.0 16.0 a 8.0 8.0 0.0 0 0 11.32 0.0 l 32.0 -32.0 a 8.0 8.0 0.0 0 0 -11.32 -11.32 Z")
        )
    }.build()
}
