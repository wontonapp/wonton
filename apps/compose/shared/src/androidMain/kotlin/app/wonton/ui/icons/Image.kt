package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Image: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Image",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 40.0 L 40.0 40.0 A 16.0 16.0 0.0 0 0 24.0 56.0 L 24.0 200.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 216.0 216.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 232.0 56.0 A 16.0 16.0 0.0 0 0 216.0 40.0 Z M 216.0 56.0 L 216.0 158.75 l -26.07 -26.06 a 16.0 16.0 0.0 0 0 -22.63 0.0 l -20.0 20.0 l -44.0 -44.0 a 16.0 16.0 0.0 0 0 -22.62 0.0 L 40.0 149.37 L 40.0 56.0 Z M 40.0 172.0 l 52.0 -52.0 l 80.0 80.0 L 40.0 200.0 Z M 216.0 200.0 L 194.63 200.0 l -36.0 -36.0 l 20.0 -20.0 L 216.0 181.38 L 216.0 200.0 Z M 144.0 100.0 a 12.0 12.0 0.0 1 1 12.0 12.0 A 12.0 12.0 0.0 0 1 144.0 100.0 Z")
        )
    }.build()
}
