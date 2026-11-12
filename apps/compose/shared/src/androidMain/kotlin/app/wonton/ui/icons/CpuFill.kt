package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.CpuFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "CpuFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 104.0 104.0 h 48.0 v 48.0 L 104.0 152.0 Z M 240.0 152.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 216.0 160.0 v 40.0 a 16.0 16.0 0.0 0 1 -16.0 16.0 L 160.0 216.0 v 16.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 L 144.0 216.0 L 112.0 216.0 v 16.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 L 96.0 216.0 L 56.0 216.0 a 16.0 16.0 0.0 0 1 -16.0 -16.0 L 40.0 160.0 L 24.0 160.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 L 40.0 144.0 L 40.0 112.0 L 24.0 112.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 L 40.0 96.0 L 40.0 56.0 A 16.0 16.0 0.0 0 1 56.0 40.0 L 96.0 40.0 L 96.0 24.0 a 8.0 8.0 0.0 0 1 16.0 0.0 L 112.0 40.0 h 32.0 L 144.0 24.0 a 8.0 8.0 0.0 0 1 16.0 0.0 L 160.0 40.0 h 40.0 a 16.0 16.0 0.0 0 1 16.0 16.0 L 216.0 96.0 h 16.0 a 8.0 8.0 0.0 0 1 0.0 16.0 L 216.0 112.0 v 32.0 h 16.0 A 8.0 8.0 0.0 0 1 240.0 152.0 Z M 168.0 96.0 a 8.0 8.0 0.0 0 0 -8.0 -8.0 L 96.0 88.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 v 64.0 a 8.0 8.0 0.0 0 0 8.0 8.0 h 64.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 Z")
        )
    }.build()
}
