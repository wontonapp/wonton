package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import app.wonton.protocol.Action
import app.wonton.protocol.TabRef

/**
 * 把“屏幕上正显示着哪个插件的页面”报给核心（[Action.SetVisiblePlugin]），核心按它自动开关端口映射，壳只报事实不判断。
 * 底栏选中的插件页、覆层和全屏覆盖层都算（[shown] 为 null 表示原生页盖住了插件宿主）；[TabRef.Url] 是公网网页，不算。
 * 配对和引导流程会整屏替换 [AppContent]，离开组合时兜底报一次看不见。
 */
@Composable
internal fun VisiblePluginReporting(tabs: TabsState, shown: TabRef?) {
    val plugin = when (shown) {
        is TabRef.Plugin -> shown.pluginId
        is TabRef.PluginPath -> shown.pluginId
        is TabRef.Bot -> shown.bot.pluginId
        is TabRef.SharedApp -> shown.pluginId
        else -> null
    }
    LaunchedEffect(plugin) { tabs.send(Action.SetVisiblePlugin(plugin)) }
    DisposableEffect(tabs) {
        onDispose { tabs.send(Action.SetVisiblePlugin(null)) }
    }
}
