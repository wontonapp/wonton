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
import app.wonton.protocol.TabRef
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 短状态条最长占一行的比例,右边留给胶囊。 */
private const val PILL_MAX_FRACTION = 0.72f

/** 插件页的浮层:短状态、接管提示和连接提示排在页面上面,右侧是一直在的胶囊。 */
@Composable
internal fun PluginPageChrome(
    tab: TabRef, tabs: PagesState, safe: PaddingValues, bannerTop: Dp, shortStatus: String?,
    capsule: PageCapsuleState, controller: PluginWebViewController, takeover: PageTakeover?,
    onClose: () -> Unit, onReload: () -> Unit, onDetails: (String) -> Unit,
    onEditPanel: (() -> Unit)? = null,
) {
    val status = LocalWontonBarStatus.current
    val colors = WontonTheme.colors
    Box(Modifier.fillMaxWidth().padding(
        start = safe.calculateLeftPadding(LayoutDirection.Ltr) + WontonTheme.spacing.s8,
        end = safe.calculateRightPadding(LayoutDirection.Ltr) + WontonTheme.spacing.s8,
        top = safe.calculateTopPadding() + WontonTheme.spacing.s8 +
            (bannerTop - safe.calculateTopPadding()).coerceAtLeast(0.dp),
    )) {
        Column(Modifier.fillMaxWidth().padding(end = WontonTheme.size.target * 2 + WontonTheme.size.hairline)) {
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
        }
        Box(Modifier.align(Alignment.TopEnd)) {
            PluginPageCapsule(tab, tabs, capsule, controller, takeover, onClose, onReload, onDetails, onEditPanel)
        }
    }
}
