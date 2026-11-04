package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.PauseFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "PauseFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 48.0 V 208.0 a 16.0 16.0 0.0 0 1 -16.0 16.0 H 160.0 a 16.0 16.0 0.0 0 1 -16.0 -16.0 V 48.0 a 16.0 16.0 0.0 0 1 16.0 -16.0 h 40.0 A 16.0 16.0 0.0 0 1 216.0 48.0 Z M 96.0 32.0 H 56.0 A 16.0 16.0 0.0 0 0 40.0 48.0 V 208.0 a 16.0 16.0 0.0 0 0 16.0 16.0 H 96.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 V 48.0 A 16.0 16.0 0.0 0 0 96.0 32.0 Z")
        )
    }.build()
}
