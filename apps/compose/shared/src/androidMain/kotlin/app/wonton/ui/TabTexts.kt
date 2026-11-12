package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalConfiguration
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabRef

@Composable
fun tabText(zh: String, en: String): String =
    if (LocalConfiguration.current.locales[0].language == "zh") zh else en

@Composable
fun TabItem.tabTitle(): String = when (tab) {
    TabRef.Chat -> tabText("聊天", "Chats")
    TabRef.Apps -> tabText("应用", "Apps")
    TabRef.Settings -> tabText("设置", "Settings")
    is TabRef.Plugin -> page?.title ?: tabText("应用页面", "App page")
}
