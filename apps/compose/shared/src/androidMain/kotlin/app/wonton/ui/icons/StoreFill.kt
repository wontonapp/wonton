package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.StoreFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "StoreFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 231.69 93.81 L 217.35 43.6 A 16.07 16.07 0.0 0 0 202.0 32.0 L 54.0 32.0 A 16.07 16.07 0.0 0 0 38.65 43.6 L 24.31 93.81 A 7.94 7.94 0.0 0 0 24.0 96.0 v 16.0 a 40.0 40.0 0.0 0 0 16.0 32.0 v 72.0 a 8.0 8.0 0.0 0 0 8.0 8.0 L 208.0 224.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 L 216.0 144.0 a 40.0 40.0 0.0 0 0 16.0 -32.0 L 232.0 96.0 A 7.94 7.94 0.0 0 0 231.69 93.81 Z M 88.0 112.0 a 24.0 24.0 0.0 0 1 -35.12 21.26 a 7.88 7.88 0.0 0 0 -1.82 -1.06 A 24.0 24.0 0.0 0 1 40.0 112.0 v -8.0 L 88.0 104.0 Z M 152.0 112.0 a 24.0 24.0 0.0 0 1 -48.0 0.0 v -8.0 h 48.0 Z M 216.0 112.0 a 24.0 24.0 0.0 0 1 -11.07 20.2 a 8.08 8.08 0.0 0 0 -1.8 1.05 A 24.0 24.0 0.0 0 1 168.0 112.0 v -8.0 h 48.0 Z")
        )
    }.build()
}
