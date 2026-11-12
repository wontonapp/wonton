package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Cpu: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Cpu",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 152.0 96.0 L 104.0 96.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 v 48.0 a 8.0 8.0 0.0 0 0 8.0 8.0 h 48.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 L 160.0 104.0 A 8.0 8.0 0.0 0 0 152.0 96.0 Z M 144.0 144.0 L 112.0 144.0 L 112.0 112.0 h 32.0 Z M 232.0 144.0 L 216.0 144.0 L 216.0 112.0 h 16.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 L 216.0 96.0 L 216.0 56.0 a 16.0 16.0 0.0 0 0 -16.0 -16.0 L 160.0 40.0 L 160.0 24.0 a 8.0 8.0 0.0 0 0 -16.0 0.0 L 144.0 40.0 L 112.0 40.0 L 112.0 24.0 a 8.0 8.0 0.0 0 0 -16.0 0.0 L 96.0 40.0 L 56.0 40.0 A 16.0 16.0 0.0 0 0 40.0 56.0 L 40.0 96.0 L 24.0 96.0 a 8.0 8.0 0.0 0 0 0.0 16.0 L 40.0 112.0 v 32.0 L 24.0 144.0 a 8.0 8.0 0.0 0 0 0.0 16.0 L 40.0 160.0 v 40.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 96.0 216.0 v 16.0 a 8.0 8.0 0.0 0 0 16.0 0.0 L 112.0 216.0 h 32.0 v 16.0 a 8.0 8.0 0.0 0 0 16.0 0.0 L 160.0 216.0 h 40.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 216.0 160.0 h 16.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 Z M 200.0 200.0 L 56.0 200.0 L 56.0 56.0 L 200.0 56.0 v 95.87 s 0.0 0.09 0.0 0.13 s 0.0 0.09 0.0 0.13 L 200.0 200.0 Z")
        )
    }.build()
}
