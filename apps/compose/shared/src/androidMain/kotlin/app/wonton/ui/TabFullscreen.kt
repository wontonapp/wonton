package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.TabDisplayChoice
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Back
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Expand
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.WontonIcon


@Composable
internal fun TabBarMenu(item: TabItem, tabs: TabsState, onEdit: () -> Unit, onDismiss: () -> Unit) {
    WontonSheet(onDismiss, item.tabTitle()) { close ->
        Column(Modifier.padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonGroup {
                item.page?.let { page ->
                    WontonRow(stringResource(R.string.tab_menu_open_fullscreen), onClick = {
                        close { tabs.send(Action.EnterFullscreen(item.tab)) }
                    })
                    Row(Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16),
                        verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(R.string.tab_menu_always_fullscreen), Modifier.weight(1f),
                            style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
                        val label = stringResource(R.string.tab_menu_always_fullscreen)
                        Box(Modifier.semantics { contentDescription = label }) {
                            WontonSwitch(page.opensFullscreen, { enabled ->
                                tabs.send(Action.SetTabDisplay(item.tab, displayChoice(enabled)))
                            })
                        }
                    }
                }
                WontonRow(stringResource(R.string.tab_edit_bar), onClick = { close(onEdit) })
            }
        }
    }
}

internal fun displayChoice(enabled: Boolean): TabDisplayChoice =
    if (enabled) TabDisplayChoice.FULLSCREEN else TabDisplayChoice.STANDARD

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun PluginTabBar(
    page: TabPage, canGoBack: Boolean, overlay: Boolean, onBack: () -> Unit,
    onClose: () -> Unit, takeover: PageTakeover?, onExpand: () -> Unit, onReload: () -> Unit,
) {
    val colors = WontonTheme.colors
    val status = LocalWontonBarStatus.current
    Column(Modifier.fillMaxWidth().background(colors.canvas.toColor())) {
        TopAppBar(
            title = { Text(status.title ?: page.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = {
                if (canGoBack) WontonIconButton(WontonIcon.Back, stringResource(R.string.tab_menu_back), onBack)
            },
            actions = {
                takeover?.let { TakeoverButton(it) }
                WontonIconButton(WontonIcon.Expand, stringResource(R.string.tab_fullscreen), onExpand)
                WontonIconButton(WontonIcon.Refresh, stringResource(R.string.tab_menu_reload), onReload)
                if (overlay) WontonIconButton(WontonIcon.Close, stringResource(R.string.common_close), onClose)
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.Transparent),
        )
        status.line?.let { Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) { it() } }
        if (page.state is TabState.Upgrading) Text(
            stringResource(R.string.tab_upgrading), Modifier.padding(horizontal = WontonTheme.spacing.pageInset),
            color = colors.textSecondary.toColor(), style = WontonTheme.typography.supporting,
        )
    }
}
