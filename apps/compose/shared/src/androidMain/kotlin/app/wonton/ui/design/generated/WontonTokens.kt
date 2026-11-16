// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber")

package app.wonton.ui.design.generated

import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
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
    val composerField = 20.dp
}

object WontonSize {
    val target = 48.dp
    val avatar = 48.dp
    val topBar = 56.dp
    val bottomBar = 64.dp
    val icon = 24.dp
    val iconSmall = 20.dp
    val iconTiny = 14.dp
    val sendButton = 36.dp
    val composerField = 40.dp
    val fileThumbnail = 44.dp
    val menuBlur = 12.dp
    val menuWidth = 200.dp
    val menuElevation = 8.dp
    val messageImage = 240.dp
    val launcherIcon = 62.dp
    val launcherCell = 76.dp
    val storeIcon = 56.dp
    val taskIcon = 40.dp
    val floatingActionButton = 56.dp
    val previewIcon = 64.dp
    val detailIcon = 112.dp
    val screenshot = 380.dp
    val chip = 36.dp
    val smallButton = 32.dp
    val switchWidth = 44.dp
    val switchHeight = 26.dp
    val switchKnob = 22.dp
    val dialogWidth = 320.dp
    val grabberWidth = 36.dp
    val grabberHeight = 4.dp
    val hairline = 1.dp
    val quoteBar = 2.dp
}

object WontonRatio {
    val pluginIconRadius = 0.2237f
    val secondaryContent = 0.7f
    val scrim = 0.4f
    val codeFill = 0.1f
}

object WontonGesture {
    val pullDistance = 96f
    val pushDistance = 64f
    val swipeDistance = 64f
    val flickDistance = 32f
    val flickVelocity = 900f
    val dragSlop = 8f
    val edgeWidth = 24f
    val timeReveal = 64f
    val pageEdgeZone = 48f
}

object WontonMotion {
    val feedback = 120
    val state = 180
    val panel = 240
    val messageInsert = 180
    val reduced = 100
}

class WontonTypography(fontFamily: FontFamily?) {
    val screenTitle = TextStyle(fontFamily = fontFamily, fontSize = 28.sp, lineHeight = 34.sp, fontWeight = FontWeight.Bold)
    val sectionTitle = TextStyle(fontFamily = fontFamily, fontSize = 20.sp, lineHeight = 26.sp, fontWeight = FontWeight.SemiBold)
    val itemTitle = TextStyle(fontFamily = fontFamily, fontSize = 17.sp, lineHeight = 24.sp, fontWeight = FontWeight.Medium)
    val body = TextStyle(fontFamily = fontFamily, fontSize = 16.sp, lineHeight = 22.sp, fontWeight = FontWeight.Normal)
    val supporting = TextStyle(fontFamily = fontFamily, fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Normal)
    val caption = TextStyle(fontFamily = fontFamily, fontSize = 12.sp, lineHeight = 16.sp, fontWeight = FontWeight.Normal)
}
