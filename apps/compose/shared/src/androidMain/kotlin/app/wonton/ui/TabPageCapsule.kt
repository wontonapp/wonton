package app.wonton.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
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
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.blur
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.platform.ViewConfiguration
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Leave
import app.wonton.ui.icons.More
import app.wonton.ui.icons.WontonIcon

private const val CAPSULE_FADE_MS = 200
private const val CAPSULE_SHADOW_ALPHA = 0.4f
private const val CAPSULE_FILL_ALPHA = 0.2f
private const val CAPSULE_DIVIDER_ALPHA = 0.2f
private val capsuleShadowRadius = 1.dp

/** 全屏页右上角的胶囊:左键开菜单,右键退出全屏;透明底,3 秒空闲淡出。 */
@Composable
internal fun PluginPageCapsule(
    tab: TabRef, page: TabPage, tabs: TabsState, state: TabCapsuleVisibility,
    controller: PluginWebViewController, takeover: PageTakeover?,
    onClose: () -> Unit, onReload: () -> Unit, onDetails: (String) -> Unit,
) {
    val opacity by animateFloatAsState(if (state.visible) 1f else 0f, tween(CAPSULE_FADE_MS), label = "capsule")
    val trailingLabel = stringResource(R.string.tab_exit_fullscreen)
    val configuration = LocalViewConfiguration.current
    // Compose expands small click targets by default; the page owns every pixel outside the visible capsule.
    val exactTargets = remember(configuration) {
        object : ViewConfiguration by configuration {
            override val minimumTouchTargetSize = DpSize.Zero
        }
    }
    CompositionLocalProvider(LocalViewConfiguration provides exactTargets) {
        Box(Modifier.size(WontonTheme.size.target * 2 + WontonTheme.size.hairline, WontonTheme.size.smallButton)) {
            if (opacity > 0f) Box(Modifier.alpha(opacity)) {
                Box(Modifier.matchParentSize().background(Color.Black.copy(alpha = CAPSULE_FILL_ALPHA),
                    RoundedCornerShape(WontonTheme.radii.pill)))
                CapsuleArtwork(WontonIcon.Leave)
            }
            if (state.visible) Row(Modifier.height(WontonTheme.size.smallButton).pointerInput(state) {
                try {
                    awaitPointerEventScope {
                        while (true) {
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            state.held = event.changes.any { it.pressed }
                            state.show()
                        }
                    }
                } finally {
                    state.held = false
                }
            }) {
                CapsuleHitTarget(stringResource(R.string.tab_page_menu)) { state.show(); state.menuOpen = true }
                Spacer(Modifier.width(WontonTheme.size.hairline))
                CapsuleHitTarget(trailingLabel) { state.show(); onClose() }
            }
            Box(Modifier.size(WontonTheme.size.target, WontonTheme.size.smallButton)) {
                CapsuleMenu(tab, page, tabs, state, controller, takeover, onReload, onDetails)
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
    tab: TabRef, page: TabPage, tabs: TabsState, state: TabCapsuleVisibility,
    controller: PluginWebViewController, takeover: PageTakeover?,
    onReload: () -> Unit, onDetails: (String) -> Unit,
) {
    val close = { state.menuOpen = false; state.show() }
    DropdownMenu(state.menuOpen, close, containerColor = WontonTheme.colors.surface.toColor()) {
        if (controller.canGoBack) DropdownMenuItem(text = { Text(stringResource(R.string.tab_menu_back)) },
            onClick = { close(); controller.goBack() })
        takeover?.let { TakeoverButton(it) }
        DropdownMenuItem(text = { Text(stringResource(R.string.tab_menu_reload)) }, onClick = { close(); onReload() })
        DropdownMenuItem(text = { Text(stringResource(R.string.tab_menu_always_fullscreen)) },
            trailingIcon = { WontonSwitch(page.opensFullscreen, { enabled ->
                close()
                tabs.send(Action.SetTabDisplay(tab, displayChoice(enabled)))
            }) }, onClick = {
                close()
                tabs.send(Action.SetTabDisplay(tab, displayChoice(!page.opensFullscreen)))
            })
        if (tab is TabRef.Plugin) DropdownMenuItem(text = { Text(stringResource(R.string.tab_menu_details)) },
            onClick = { close(); onDetails(tab.pluginId) })
    }
}
