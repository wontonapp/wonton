package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Share: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Share",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 112.0 v 96.0 a 16.0 16.0 0.0 0 1 -16.0 16.0 H 56.0 a 16.0 16.0 0.0 0 1 -16.0 -16.0 V 112.0 A 16.0 16.0 0.0 0 1 56.0 96.0 H 80.0 a 8.0 8.0 0.0 0 1 0.0 16.0 H 56.0 v 96.0 H 200.0 V 112.0 H 176.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 24.0 A 16.0 16.0 0.0 0 1 216.0 112.0 Z M 93.66 69.66 L 120.0 43.31 V 136.0 a 8.0 8.0 0.0 0 0 16.0 0.0 V 43.31 l 26.34 26.35 a 8.0 8.0 0.0 0 0 11.32 -11.32 l -40.0 -40.0 a 8.0 8.0 0.0 0 0 -11.32 0.0 l -40.0 40.0 A 8.0 8.0 0.0 0 0 93.66 69.66 Z")
        )
    }.build()
}
