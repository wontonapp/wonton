package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Expand: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Expand",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 48.0 L 216.0 88.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 L 200.0 56.0 L 168.0 56.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 40.0 A 8.0 8.0 0.0 0 1 216.0 48.0 Z M 88.0 200.0 L 56.0 200.0 L 56.0 168.0 a 8.0 8.0 0.0 0 0 -16.0 0.0 v 40.0 a 8.0 8.0 0.0 0 0 8.0 8.0 L 88.0 216.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 Z M 208.0 160.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 v 32.0 L 168.0 200.0 a 8.0 8.0 0.0 0 0 0.0 16.0 h 40.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 L 216.0 168.0 A 8.0 8.0 0.0 0 0 208.0 160.0 Z M 88.0 40.0 L 48.0 40.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 L 40.0 88.0 a 8.0 8.0 0.0 0 0 16.0 0.0 L 56.0 56.0 L 88.0 56.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 Z")
        )
    }.build()
}
