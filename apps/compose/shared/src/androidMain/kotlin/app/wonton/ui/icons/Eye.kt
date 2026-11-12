package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Eye: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Eye",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 247.31 124.76 c -0.35 -0.79 -8.82 -19.58 -27.65 -38.41 C 194.57 61.26 162.88 48.0 128.0 48.0 S 61.43 61.26 36.34 86.35 C 17.51 105.18 9.0 124.0 8.69 124.76 a 8.0 8.0 0.0 0 0 0.0 6.5 c 0.35 0.79 8.82 19.57 27.65 38.4 C 61.43 194.74 93.12 208.0 128.0 208.0 s 66.57 -13.26 91.66 -38.34 c 18.83 -18.83 27.3 -37.61 27.65 -38.4 A 8.0 8.0 0.0 0 0 247.31 124.76 Z M 128.0 192.0 c -30.78 0.0 -57.67 -11.19 -79.93 -33.25 A 133.47 133.47 0.0 0 1 25.0 128.0 A 133.33 133.33 0.0 0 1 48.07 97.25 C 70.33 75.19 97.22 64.0 128.0 64.0 s 57.67 11.19 79.93 33.25 A 133.46 133.46 0.0 0 1 231.05 128.0 C 223.84 141.46 192.43 192.0 128.0 192.0 Z M 128.0 80.0 a 48.0 48.0 0.0 1 0 48.0 48.0 A 48.05 48.05 0.0 0 0 128.0 80.0 Z M 128.0 160.0 a 32.0 32.0 0.0 1 1 32.0 -32.0 A 32.0 32.0 0.0 0 1 128.0 160.0 Z")
        )
    }.build()
}
