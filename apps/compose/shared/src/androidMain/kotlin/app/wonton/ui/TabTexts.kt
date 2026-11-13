package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.OpenPage
import app.wonton.protocol.TabRef

@Composable
fun OpenPage.tabTitle(): String = when (tab) {
    TabRef.Chat -> stringResource(R.string.tab_chat)
    TabRef.Apps -> stringResource(R.string.tab_apps)
    is TabRef.Plugin, is TabRef.PluginPath, is TabRef.Bot, is TabRef.Url, is TabRef.SharedApp,
    is TabRef.UiResource, is TabRef.Widget -> page?.title
        ?: stringResource(R.string.tab_app_page)
}
