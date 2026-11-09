package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.PinFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "PinFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 235.33 104.0 l -53.47 53.65 c 4.56 12.67 6.45 33.89 -13.19 60.0 A 15.93 15.93 0.0 0 1 157.0 224.0 c -0.38 0.0 -0.75 0.0 -1.13 0.0 a 16.0 16.0 0.0 0 1 -11.32 -4.69 L 96.29 171.0 L 53.66 213.66 a 8.0 8.0 0.0 0 1 -11.32 -11.32 L 85.0 159.71 l -48.3 -48.3 A 16.0 16.0 0.0 0 1 38.0 87.63 c 25.42 -20.51 49.75 -16.48 60.4 -13.14 L 152.0 20.7 a 16.0 16.0 0.0 0 1 22.63 0.0 l 60.69 60.68 A 16.0 16.0 0.0 0 1 235.33 104.0 Z")
        )
    }.build()
}
