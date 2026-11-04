package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Palette: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Palette",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 200.77 53.89 A 103.27 103.27 0.0 0 0 128.0 24.0 h -1.07 A 104.0 104.0 0.0 0 0 24.0 128.0 c 0.0 43.0 26.58 79.06 69.36 94.17 A 32.0 32.0 0.0 0 0 136.0 192.0 a 16.0 16.0 0.0 0 1 16.0 -16.0 h 46.21 a 31.81 31.81 0.0 0 0 31.2 -24.88 a 104.43 104.43 0.0 0 0 2.59 -24.0 A 103.28 103.28 0.0 0 0 200.77 53.89 Z M 213.77 147.6 A 15.89 15.89 0.0 0 1 198.21 160.0 L 152.0 160.0 a 32.0 32.0 0.0 0 0 -32.0 32.0 a 16.0 16.0 0.0 0 1 -21.31 15.07 C 62.49 194.3 40.0 164.0 40.0 128.0 a 88.0 88.0 0.0 0 1 87.09 -88.0 h 0.9 a 88.35 88.35 0.0 0 1 88.0 87.25 A 88.86 88.86 0.0 0 1 213.81 147.6 Z M 140.0 76.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 140.0 76.0 Z M 96.0 100.0 A 12.0 12.0 0.0 1 1 84.0 88.0 A 12.0 12.0 0.0 0 1 96.0 100.0 Z M 96.0 156.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 96.0 156.0 Z M 184.0 100.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 184.0 100.0 Z")
        )
    }.build()
}
