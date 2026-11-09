package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Document: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Document",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 213.66 82.34 l -56.0 -56.0 A 8.0 8.0 0.0 0 0 152.0 24.0 L 56.0 24.0 A 16.0 16.0 0.0 0 0 40.0 40.0 L 40.0 216.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 200.0 232.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 216.0 88.0 A 8.0 8.0 0.0 0 0 213.66 82.34 Z M 160.0 51.31 L 188.69 80.0 L 160.0 80.0 Z M 200.0 216.0 L 56.0 216.0 L 56.0 40.0 h 88.0 L 144.0 88.0 a 8.0 8.0 0.0 0 0 8.0 8.0 h 48.0 L 200.0 216.0 Z M 168.0 136.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 96.0 144.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 64.0 A 8.0 8.0 0.0 0 1 168.0 136.0 Z M 168.0 168.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 96.0 176.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 64.0 A 8.0 8.0 0.0 0 1 168.0 168.0 Z")
        )
    }.build()
}
