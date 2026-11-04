package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.File: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "File",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 213.66 82.34 l -56.0 -56.0 A 8.0 8.0 0.0 0 0 152.0 24.0 H 56.0 A 16.0 16.0 0.0 0 0 40.0 40.0 V 216.0 a 16.0 16.0 0.0 0 0 16.0 16.0 H 200.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 V 88.0 A 8.0 8.0 0.0 0 0 213.66 82.34 Z M 160.0 51.31 L 188.69 80.0 H 160.0 Z M 200.0 216.0 H 56.0 V 40.0 h 88.0 V 88.0 a 8.0 8.0 0.0 0 0 8.0 8.0 h 48.0 V 216.0 Z")
        )
    }.build()
}
