package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Help: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Help",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 140.0 180.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 140.0 180.0 Z M 128.0 72.0 c -22.06 0.0 -40.0 16.15 -40.0 36.0 v 4.0 a 8.0 8.0 0.0 0 0 16.0 0.0 v -4.0 c 0.0 -11.0 10.77 -20.0 24.0 -20.0 s 24.0 9.0 24.0 20.0 s -10.77 20.0 -24.0 20.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 v 8.0 a 8.0 8.0 0.0 0 0 16.0 0.0 v -0.72 c 18.24 -3.35 32.0 -17.9 32.0 -35.28 C 168.0 88.15 150.06 72.0 128.0 72.0 Z M 232.0 128.0 A 104.0 104.0 0.0 1 1 128.0 24.0 A 104.11 104.11 0.0 0 1 232.0 128.0 Z M 216.0 128.0 a 88.0 88.0 0.0 1 0 -88.0 88.0 A 88.1 88.1 0.0 0 0 216.0 128.0 Z")
        )
    }.build()
}
