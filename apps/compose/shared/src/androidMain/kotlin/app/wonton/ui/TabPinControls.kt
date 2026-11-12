package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabsViewModel
import app.wonton.ui.design.TabBarIcon
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun TabPinSwitch(choice: TabChoice, checked: Boolean, enabled: Boolean) {
    val tabs = LocalTabs.current
    val label = tabText("固定到底栏", "Pin to tab bar")
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = WontonTheme.spacing.s16),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(choice.title, Modifier.weight(1f), style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor())
        Column(Modifier.semantics { contentDescription = "${choice.title}, $label" }) {
            WontonSwitch(checked, { on ->
                if (on) tabs.pin(choice.tab, choice.title) else tabs.send(Action.UnpinTab(choice.tab))
            }, enabled)
        }
    }
}

@Composable
fun PluginTabPinControls(plugin: InstalledPlugin) {
    val tabs = LocalTabs.current
    val choices = plugin.tabChoices
    if (choices.isEmpty()) return
    DetailSection(tabText("固定到底栏", "Pin to tab bar")) {
        WontonGroup {
            choices.forEach { choice ->
                val item = tabs.viewModel.items.firstOrNull { it.tab == choice.tab }
                TabPinSwitch(choice, checked = item != null, enabled = item?.removable ?: true)
            }
        }
    }
}

/** 安装完成页使用按钮，只有尚未固定的页面可选。 */
@Composable
fun PluginTabPinButton(plugin: InstalledPlugin) {
    val tabs = LocalTabs.current
    val choices = plugin.tabChoices.filter { choice -> tabs.viewModel.items.none { it.tab == choice.tab } }
    var choosingPage by remember(plugin.id) { mutableStateOf(false) }
    if (choices.isEmpty()) return
    WontonButton(tabText("固定到底栏", "Pin to tab bar"), {
        if (choices.size == 1) tabs.pin(choices.single().tab, choices.single().title)
        else choosingPage = true
    })
    if (choosingPage) TabPageChoiceSheet(plugin) { choosingPage = false }
}

/** 启动器长按菜单有多页时，先让用户选页面；单页可由调用方直接 pin。 */
@Composable
fun TabPageChoiceSheet(plugin: InstalledPlugin, onDismiss: () -> Unit) {
    val tabs = LocalTabs.current
    WontonSheet(onDismiss, tabText("选择页面", "Choose a page")) { close ->
        Column(Modifier.verticalScroll(rememberScrollState())) {
            val available = plugin.tabChoices.filter { choice -> tabs.viewModel.items.none { it.tab == choice.tab } }
            available.forEach { choice ->
                WontonRow(choice.title, onClick = { close { tabs.pin(choice.tab, choice.title) } })
            }
        }
    }
}

/** 只有核心 full=true 才展示替换面板；未满由 TabsState.pin 直接发送。 */
@Composable
fun TabPinSheet(viewModel: TabsViewModel, request: PinRequest, onDismiss: () -> Unit) {
    val tabs = LocalTabs.current
    var rejection by remember(request) { mutableStateOf<Problem?>(null) }
    WontonSheet(onDismiss, tabText("选择要替换的一项", "Choose a tab to replace")) { close ->
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
            .padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
            TabsConfigurationProblem(viewModel)
            rejection?.let { ProblemNotice(it) }
            Text(tabText("底栏已满：固定${request.title}", "Tab bar full: pin ${request.title}"),
                style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
            WontonGroup {
                viewModel.items.filter { it.removable && it.tab != request.tab }.forEach { item ->
                    TabReplacementRow(item) {
                        rejection = tabs.core.send(Action.PinTab(request.tab, item.tab))
                        if (rejection == null) close {}
                    }
                }
            }
            WontonRow(tabText("取消", "Cancel"), onClick = { close {} })
        }
    }
}

@Composable
private fun TabReplacementRow(item: TabItem, onClick: () -> Unit) {
    val tabs = LocalTabs.current
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        TabBarIcon(item, false, colors.textPrimary.toColor(), tabs.plugins)
        Text(item.tabTitle(), style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
    }
}
