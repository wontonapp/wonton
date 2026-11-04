// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber")

package app.wonton.ui.design.generated

import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

object WontonSpacing {
    val s4 = 4.dp
    val s8 = 8.dp
    val s12 = 12.dp
    val s16 = 16.dp
    val s24 = 24.dp
    val s32 = 32.dp
    val s48 = 48.dp
    val pageInset = 16.dp
    val sectionGap = 24.dp
}

object WontonRadii {
    val control = 12.dp
    val card = 16.dp
    val bubble = 16.dp
    val bubbleJoined = 4.dp
    val pill = 999.dp
    val bar = 22.dp
    val barItem = 16.dp
}

object WontonSize {
    val target = 48.dp
    val topBar = 56.dp
    val bottomBar = 64.dp
    val icon = 24.dp
    val iconSmall = 20.dp
    val launcherIcon = 62.dp
    val launcherCell = 76.dp
    val hairline = 1.dp
}

object WontonRatio {
    val pluginIconRadius = 0.2237f
}

object WontonMotion {
    val feedback = 120
    val state = 180
    val panel = 240
    val messageInsert = 180
    val reduced = 100
}

object WontonTypography {
    val screenTitle = TextStyle(fontSize = 28.sp, lineHeight = 34.sp, fontWeight = FontWeight.Bold)
    val sectionTitle = TextStyle(fontSize = 20.sp, lineHeight = 26.sp, fontWeight = FontWeight.SemiBold)
    val itemTitle = TextStyle(fontSize = 17.sp, lineHeight = 24.sp, fontWeight = FontWeight.Medium)
    val body = TextStyle(fontSize = 16.sp, lineHeight = 22.sp, fontWeight = FontWeight.Normal)
    val supporting = TextStyle(fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Normal)
    val caption = TextStyle(fontSize = 12.sp, lineHeight = 16.sp, fontWeight = FontWeight.Normal)
}
