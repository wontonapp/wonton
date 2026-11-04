package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.CheckCircle: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "CheckCircle",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 173.66 98.34 a 8.0 8.0 0.0 0 1 0.0 11.32 l -56.0 56.0 a 8.0 8.0 0.0 0 1 -11.32 0.0 l -24.0 -24.0 a 8.0 8.0 0.0 0 1 11.32 -11.32 L 112.0 148.69 l 50.34 -50.35 A 8.0 8.0 0.0 0 1 173.66 98.34 Z M 232.0 128.0 A 104.0 104.0 0.0 1 1 128.0 24.0 A 104.11 104.11 0.0 0 1 232.0 128.0 Z M 216.0 128.0 a 88.0 88.0 0.0 1 0 -88.0 88.0 A 88.1 88.1 0.0 0 0 216.0 128.0 Z")
        )
    }.build()
}
