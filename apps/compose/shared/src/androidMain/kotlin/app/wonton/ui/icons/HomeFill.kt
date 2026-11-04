package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.HomeFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "HomeFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 224.0 120.0 v 96.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 H 160.0 a 8.0 8.0 0.0 0 1 -8.0 -8.0 V 164.0 a 4.0 4.0 0.0 0 0 -4.0 -4.0 H 108.0 a 4.0 4.0 0.0 0 0 -4.0 4.0 v 52.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 H 40.0 a 8.0 8.0 0.0 0 1 -8.0 -8.0 V 120.0 a 16.0 16.0 0.0 0 1 4.69 -11.31 l 80.0 -80.0 a 16.0 16.0 0.0 0 1 22.62 0.0 l 80.0 80.0 A 16.0 16.0 0.0 0 1 224.0 120.0 Z")
        )
    }.build()
}
