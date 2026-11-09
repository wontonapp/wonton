package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Pause: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Pause",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 200.0 32.0 L 160.0 32.0 a 16.0 16.0 0.0 0 0 -16.0 16.0 L 144.0 208.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 40.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 216.0 48.0 A 16.0 16.0 0.0 0 0 200.0 32.0 Z M 200.0 208.0 L 160.0 208.0 L 160.0 48.0 h 40.0 Z M 96.0 32.0 L 56.0 32.0 A 16.0 16.0 0.0 0 0 40.0 48.0 L 40.0 208.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 96.0 224.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 112.0 48.0 A 16.0 16.0 0.0 0 0 96.0 32.0 Z M 96.0 208.0 L 56.0 208.0 L 56.0 48.0 L 96.0 48.0 Z")
        )
    }.build()
}
