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
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.platform.ViewConfiguration
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.DpSize
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.protocol.MyAppRef
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Leave
import app.wonton.ui.icons.More
import app.wonton.ui.icons.WontonIcon

/** 页面右上角的 Material 导航胶囊；菜单和退出常驻，不随网页滚动隐藏。 */
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
        Surface(
            modifier = Modifier.size(
                WontonTheme.size.target * 2 + WontonTheme.size.hairline,
                WontonTheme.size.smallButton,
            ),
            color = WontonTheme.colors.surface.toColor(),
            shape = RoundedCornerShape(WontonTheme.radii.pill),
        ) {
            Box {
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
}

@Composable
private fun CapsuleArtwork(trailingIcon: ImageVector) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        CapsuleGlyph(WontonIcon.More)
        Box(Modifier.width(WontonTheme.size.hairline).height(WontonTheme.size.iconSmall)
            .background(WontonTheme.colors.separator.toColor()))
        CapsuleGlyph(trailingIcon)
    }
}

@Composable
private fun CapsuleGlyph(icon: ImageVector) {
    Box(Modifier.size(WontonTheme.size.target, WontonTheme.size.smallButton), contentAlignment = Alignment.Center) {
        // 装饰：旁边叠着的命中区自己带名字。
        Icon(icon, null, Modifier.size(WontonTheme.size.iconSmall), tint = WontonTheme.colors.textPrimary.toColor())
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
                    tabs.send(Action.Pages(PageAction.AddMyApp(MyAppRef.Page(tab), null, null)))
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
