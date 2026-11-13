package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.ui.draw.blur
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.platform.ViewConfiguration
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.MyAppRef
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Leave
import app.wonton.ui.icons.More
import app.wonton.ui.icons.WontonIcon

private const val CAPSULE_FILL_ALPHA = 0.2f
private const val CAPSULE_SHADOW_ALPHA = 0.4f
private const val CAPSULE_DIVIDER_ALPHA = 0.2f
private val capsuleShadowRadius = 1.dp

/** 胶囊上的菜单开没开；胶囊本身一直显示，不随滚动或空闲隐藏。 */
internal class PageCapsuleState {
    var menuOpen by mutableStateOf(false)
}

/** 页面右上角的胶囊:左键开菜单,右键关掉页面;透明底,一直在。 */
@Composable
internal fun PluginPageCapsule(
    tab: TabRef, tabs: PagesState, state: PageCapsuleState,
    controller: PluginWebViewController, takeover: PageTakeover?,
    onClose: () -> Unit, onReload: () -> Unit, onDetails: (String) -> Unit,
    onEditPanel: (() -> Unit)? = null,
) {
    val configuration = LocalViewConfiguration.current
    // Compose expands small click targets by default; the page owns every pixel outside the visible capsule.
    val exactTargets = remember(configuration) {
        object : ViewConfiguration by configuration {
            override val minimumTouchTargetSize = DpSize.Zero
        }
    }
    CompositionLocalProvider(LocalViewConfiguration provides exactTargets) {
        Box(Modifier.size(WontonTheme.size.target * 2 + WontonTheme.size.hairline, WontonTheme.size.smallButton)) {
            Box(Modifier.matchParentSize().background(Color.Black.copy(alpha = CAPSULE_FILL_ALPHA),
                RoundedCornerShape(WontonTheme.radii.pill)))
            CapsuleArtwork(WontonIcon.Leave)
            Row(Modifier.height(WontonTheme.size.smallButton)) {
                CapsuleHitTarget(stringResource(R.string.tab_page_menu)) { state.menuOpen = true }
                Spacer(Modifier.width(WontonTheme.size.hairline))
                CapsuleHitTarget(stringResource(R.string.tab_page_close), onClose)
            }
            Box(Modifier.size(WontonTheme.size.target, WontonTheme.size.smallButton)) {
                CapsuleMenu(tab, tabs, state, controller, takeover, onReload, onDetails, onEditPanel)
            }
        }
    }
}

@Composable
private fun CapsuleArtwork(trailingIcon: ImageVector) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        CapsuleGlyph(WontonIcon.More)
        Box(Modifier.width(WontonTheme.size.hairline).height(WontonTheme.size.iconSmall)
            .background(Color.White.copy(alpha = CAPSULE_DIVIDER_ALPHA)))
        CapsuleGlyph(trailingIcon)
    }
}

@Composable
private fun CapsuleGlyph(icon: ImageVector) {
    Box(Modifier.size(WontonTheme.size.target, WontonTheme.size.smallButton), contentAlignment = Alignment.Center) {
        // 装饰：这是同一个图形的阴影层。
        Icon(icon, null, Modifier.size(WontonTheme.size.iconSmall)
            .blur(capsuleShadowRadius, BlurredEdgeTreatment.Unbounded),
            tint = Color.Black.copy(alpha = CAPSULE_SHADOW_ALPHA))
        // 装饰：旁边叠着的命中区自己带名字。
        Icon(icon, null, Modifier.size(WontonTheme.size.iconSmall), tint = Color.White)
    }
}

@Composable
private fun CapsuleHitTarget(label: String, onClick: () -> Unit) {
    Box(Modifier.size(WontonTheme.size.target, WontonTheme.size.smallButton)
        .clickable(role = Role.Button, onClick = onClick).semantics { contentDescription = label })
}

@Composable
private fun CapsuleMenu(
    tab: TabRef, tabs: PagesState, state: PageCapsuleState,
    controller: PluginWebViewController, takeover: PageTakeover?,
    onReload: () -> Unit, onDetails: (String) -> Unit, onEditPanel: (() -> Unit)?,
) {
    val close = { state.menuOpen = false }
    DropdownMenu(state.menuOpen, close, containerColor = WontonTheme.colors.surface.toColor()) {
        if (controller.canGoBack) DropdownMenuItem(text = { Text(stringResource(R.string.tab_menu_back)) },
            onClick = { close(); controller.goBack() })
        takeover?.let { TakeoverButton(it) }
        DropdownMenuItem(text = { Text(stringResource(R.string.tab_menu_reload)) }, onClick = { close(); onReload() })
        if (tab is TabRef.Plugin) {
            DropdownMenuItem(text = { Text(stringResource(R.string.myapp_add)) },
                onClick = {
                    close()
                    tabs.send(Action.AddMyApp(MyAppRef.Page(tab), null, null))
                })
            DropdownMenuItem(text = { Text(stringResource(R.string.tab_menu_details)) },
                onClick = { close(); onDetails(tab.pluginId) })
        }
        if (onEditPanel != null) {
            DropdownMenuItem(text = { Text(stringResource(R.string.panel_edit)) },
                onClick = { close(); onEditPanel() })
        }
    }
}
