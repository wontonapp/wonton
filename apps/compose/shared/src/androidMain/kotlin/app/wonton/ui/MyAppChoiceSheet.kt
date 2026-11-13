package app.wonton.ui

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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.MyAppRef
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
/** 多页插件挑一页加进"我的应用"；只有一个可加的页面时不开这个面板。 */
@Composable
internal fun MyAppChoiceSheet(plugin: InstalledPlugin, onDismiss: () -> Unit) {
    val pages = LocalPages.current
    WontonSheet(onDismiss, stringResource(R.string.tab_choose_page)) { close ->
        Column(Modifier.verticalScroll(rememberScrollState())) {
            val mine = pages.plugins.viewModel.sidebar.apps.mapNotNull { it.tab }.toSet()
            val available = plugin.tabChoices.filter { choice -> choice.tab !in mine }
            available.forEach { choice ->
                WontonRow(choice.title, onClick = {
                    close { pages.send(Action.AddMyApp(MyAppRef.Page(choice.tab), null, null)) }
                })
            }
        }
    }
}

/** 装好后的完成页：把刚装的加进"我的应用"；多页的先挑一页。 */
@Composable
internal fun MyAppAddButton(plugin: InstalledPlugin) {
    val pages = LocalPages.current
    var choosing by remember { mutableStateOf(false) }
    WontonButton(stringResource(R.string.myapp_add), {
        val mine = pages.plugins.viewModel.sidebar.apps.mapNotNull { it.tab }.toSet()
        val choices = plugin.tabChoices.filter { it.tab !in mine }
        if (choices.size == 1) pages.send(Action.AddMyApp(MyAppRef.Page(choices.single().tab), null, null))
        else if (choices.isNotEmpty()) choosing = true
    })
    if (choosing) MyAppChoiceSheet(plugin) { choosing = false }
}

/** 插件详情页里的"我的应用":每页一行开关,加进或移出。 */
@Composable
internal fun MyAppsControls(plugin: InstalledPlugin) {
    val pages = LocalPages.current
    val choices = plugin.tabChoices
    if (choices.isEmpty()) return
    DetailSection(stringResource(R.string.settings_my_apps)) {
        WontonGroup {
            choices.forEach { choice ->
                val mine = pages.plugins.viewModel.sidebar.apps.any { it.tab == choice.tab }
                Row(
                    Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
                        .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                ) {
                    Text(choice.title, Modifier.weight(1f), style = WontonTheme.typography.body,
                        color = WontonTheme.colors.textPrimary.toColor())
                    WontonSwitch(mine, { on ->
                        if (on) pages.send(Action.AddMyApp(MyAppRef.Page(choice.tab), null, null))
                        else pages.send(Action.RemoveMyApp(MyAppRef.Page(choice.tab)))
                    })
                }
            }
        }
    }
}
