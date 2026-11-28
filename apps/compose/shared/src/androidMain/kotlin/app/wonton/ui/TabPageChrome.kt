package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 短状态条最长占一行的比例:它是一句短话,不横贯整页。 */
private const val PILL_MAX_FRACTION = 0.72f

/**
 * 页面上的浮层:短状态、接管提示和连接提示排在状态栏下沿下面,浮在网页上;一样都没有时顶上什么都不浮。
 * [capsule] 只有公网网页给(右上角的胶囊),给了的话左边这一列让开它。
 */
@Composable
internal fun PluginPageChrome(
    safe: PaddingValues, bannerTop: Dp, shortStatus: String?, takeover: PageTakeover?,
    hint: String? = null, capsule: (@Composable () -> Unit)? = null,
) {
    val status = LocalWontonBarStatus.current
    val colors = WontonTheme.colors
    Box(Modifier.fillMaxWidth().padding(
        start = safe.calculateLeftPadding(LayoutDirection.Ltr) + WontonTheme.spacing.s8,
        end = safe.calculateRightPadding(LayoutDirection.Ltr) + WontonTheme.spacing.s8,
        top = safe.calculateTopPadding() + WontonTheme.spacing.s8 +
            (bannerTop - safe.calculateTopPadding()).coerceAtLeast(0.dp),
    )) {
        val capsuleWidth = if (capsule != null) WontonTheme.size.target * 2 + WontonTheme.size.hairline else 0.dp
        Column(Modifier.fillMaxWidth().padding(end = capsuleWidth)) {
            if (shortStatus != null) {
                Surface(Modifier.fillMaxWidth(PILL_MAX_FRACTION), shape = RoundedCornerShape(WontonTheme.radii.control),
                    color = colors.surface.toColor()) {
                    Text(shortStatus, Modifier.padding(WontonTheme.spacing.s8),
                        style = WontonTheme.typography.supporting, color = colors.textPrimary.toColor(),
                        maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            status.line?.let { line ->
                Surface(color = colors.surface.toColor(), shape = RoundedCornerShape(WontonTheme.radii.control)) {
                    Box(Modifier.padding(WontonTheme.spacing.s8)) { line() }
                }
            }
            takeover?.let { TakeoverNotice(it) }
            hint?.let { PageHint(it) }
        }
        capsule?.let { Box(Modifier.align(Alignment.TopEnd)) { it() } }
    }
}
