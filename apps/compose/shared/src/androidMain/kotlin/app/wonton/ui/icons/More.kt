package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.More: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "More",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 140.0 128.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 140.0 128.0 Z M 196.0 116.0 a 12.0 12.0 0.0 1 0 12.0 12.0 A 12.0 12.0 0.0 0 0 196.0 116.0 Z M 60.0 116.0 a 12.0 12.0 0.0 1 0 12.0 12.0 A 12.0 12.0 0.0 0 0 60.0 116.0 Z")
        )
    }.build()
}
