package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Search: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Search",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 229.66 218.34 l -50.07 -50.06 a 88.11 88.11 0.0 1 0 -11.31 11.31 l 50.06 50.07 a 8.0 8.0 0.0 0 0 11.32 -11.32 Z M 40.0 112.0 a 72.0 72.0 0.0 1 1 72.0 72.0 A 72.08 72.08 0.0 0 1 40.0 112.0 Z")
        )
    }.build()
}
