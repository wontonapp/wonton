package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabRef

@Composable
fun TabItem.tabTitle(): String = when (tab) {
    TabRef.Chat -> stringResource(R.string.tab_chat)
    TabRef.Apps -> stringResource(R.string.tab_apps)
    TabRef.Settings -> stringResource(R.string.tab_settings)
    is TabRef.Plugin -> page?.title ?: stringResource(R.string.tab_app_page)
}
