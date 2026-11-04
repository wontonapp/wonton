package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Upload: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Upload",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 224.0 144.0 v 64.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 H 40.0 a 8.0 8.0 0.0 0 1 -8.0 -8.0 V 144.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 56.0 H 208.0 V 144.0 a 8.0 8.0 0.0 0 1 16.0 0.0 Z M 93.66 77.66 L 120.0 51.31 V 144.0 a 8.0 8.0 0.0 0 0 16.0 0.0 V 51.31 l 26.34 26.35 a 8.0 8.0 0.0 0 0 11.32 -11.32 l -40.0 -40.0 a 8.0 8.0 0.0 0 0 -11.32 0.0 l -40.0 40.0 A 8.0 8.0 0.0 0 0 93.66 77.66 Z")
        )
    }.build()
}
