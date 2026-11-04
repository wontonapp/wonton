package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Store: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Store",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 232.0 96.0 a 7.89 7.89 0.0 0 0 -0.3 -2.2 L 217.35 43.6 A 16.07 16.07 0.0 0 0 202.0 32.0 L 54.0 32.0 A 16.07 16.07 0.0 0 0 38.65 43.6 L 24.31 93.8 A 7.89 7.89 0.0 0 0 24.0 96.0 h 0.0 v 16.0 a 40.0 40.0 0.0 0 0 16.0 32.0 v 72.0 a 8.0 8.0 0.0 0 0 8.0 8.0 L 208.0 224.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 L 216.0 144.0 a 40.0 40.0 0.0 0 0 16.0 -32.0 L 232.0 96.0 Z M 54.0 48.0 L 202.0 48.0 l 11.42 40.0 L 42.61 88.0 Z M 104.0 104.0 h 48.0 v 8.0 a 24.0 24.0 0.0 0 1 -48.0 0.0 Z M 88.0 104.0 v 8.0 a 24.0 24.0 0.0 0 1 -35.12 21.26 a 7.88 7.88 0.0 0 0 -1.82 -1.06 A 24.0 24.0 0.0 0 1 40.0 112.0 v -8.0 Z M 200.0 208.0 L 56.0 208.0 L 56.0 151.2 a 40.57 40.57 0.0 0 0 8.0 0.8 a 40.0 40.0 0.0 0 0 32.0 -16.0 a 40.0 40.0 0.0 0 0 64.0 0.0 a 40.0 40.0 0.0 0 0 32.0 16.0 a 40.57 40.57 0.0 0 0 8.0 -0.8 Z M 204.93 132.2 a 8.08 8.08 0.0 0 0 -1.8 1.05 A 24.0 24.0 0.0 0 1 168.0 112.0 v -8.0 h 48.0 v 8.0 A 24.0 24.0 0.0 0 1 204.93 132.2 Z")
        )
    }.build()
}
