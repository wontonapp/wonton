package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.WrenchFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "WrenchFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 232.0 96.0 a 72.0 72.0 0.0 0 1 -100.94 66.0 L 79.0 222.22 c -0.12 0.14 -0.26 0.29 -0.39 0.42 a 32.0 32.0 0.0 0 1 -45.26 -45.26 c 0.14 -0.13 0.28 -0.27 0.43 -0.39 L 94.0 124.94 a 72.07 72.07 0.0 0 1 83.54 -98.78 a 8.0 8.0 0.0 0 1 3.93 13.19 L 144.0 80.0 l 5.66 26.35 L 176.0 112.0 l 40.65 -37.52 a 8.0 8.0 0.0 0 1 13.19 3.93 A 72.6 72.6 0.0 0 1 232.0 96.0 Z")
        )
    }.build()
}
