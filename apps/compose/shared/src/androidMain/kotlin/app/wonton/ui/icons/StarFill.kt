package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.StarFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "StarFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 234.29 114.85 l -45.0 38.83 L 203.0 211.75 a 16.4 16.4 0.0 0 1 -24.5 17.82 L 128.0 198.49 L 77.47 229.57 A 16.4 16.4 0.0 0 1 53.0 211.75 l 13.76 -58.07 l -45.0 -38.83 A 16.46 16.46 0.0 0 1 31.08 86.0 l 59.0 -4.76 l 22.76 -55.08 a 16.36 16.36 0.0 0 1 30.27 0.0 l 22.75 55.08 l 59.0 4.76 a 16.46 16.46 0.0 0 1 9.37 28.86 Z")
        )
    }.build()
}
