package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Inbox: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Inbox",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 221.8 175.94 C 216.25 166.38 208.0 139.33 208.0 104.0 a 80.0 80.0 0.0 1 0 -160.0 0.0 c 0.0 35.34 -8.26 62.38 -13.81 71.94 A 16.0 16.0 0.0 0 0 48.0 200.0 H 88.81 a 40.0 40.0 0.0 0 0 78.38 0.0 H 208.0 a 16.0 16.0 0.0 0 0 13.8 -24.06 Z M 128.0 216.0 a 24.0 24.0 0.0 0 1 -22.62 -16.0 h 45.24 A 24.0 24.0 0.0 0 1 128.0 216.0 Z M 48.0 184.0 c 7.7 -13.24 16.0 -43.92 16.0 -80.0 a 64.0 64.0 0.0 1 1 128.0 0.0 c 0.0 36.05 8.28 66.73 16.0 80.0 Z")
        )
    }.build()
}
