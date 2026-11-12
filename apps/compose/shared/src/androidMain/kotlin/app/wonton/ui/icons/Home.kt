package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Home: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Home",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 219.31 108.68 l -80.0 -80.0 a 16.0 16.0 0.0 0 0 -22.62 0.0 l -80.0 80.0 A 15.87 15.87 0.0 0 0 32.0 120.0 v 96.0 a 8.0 8.0 0.0 0 0 8.0 8.0 h 64.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 V 160.0 h 32.0 v 56.0 a 8.0 8.0 0.0 0 0 8.0 8.0 h 64.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 V 120.0 A 15.87 15.87 0.0 0 0 219.31 108.68 Z M 208.0 208.0 H 160.0 V 152.0 a 8.0 8.0 0.0 0 0 -8.0 -8.0 H 104.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 v 56.0 H 48.0 V 120.0 l 80.0 -80.0 l 80.0 80.0 Z")
        )
    }.build()
}
