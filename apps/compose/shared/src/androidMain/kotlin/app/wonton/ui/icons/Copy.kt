package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Copy: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Copy",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 32.0 L 88.0 32.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 L 80.0 80.0 L 40.0 80.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 L 32.0 216.0 a 8.0 8.0 0.0 0 0 8.0 8.0 L 168.0 224.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 L 176.0 176.0 h 40.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 L 224.0 40.0 A 8.0 8.0 0.0 0 0 216.0 32.0 Z M 160.0 208.0 L 48.0 208.0 L 48.0 96.0 L 160.0 96.0 Z M 208.0 160.0 L 176.0 160.0 L 176.0 88.0 a 8.0 8.0 0.0 0 0 -8.0 -8.0 L 96.0 80.0 L 96.0 48.0 L 208.0 48.0 Z")
        )
    }.build()
}
