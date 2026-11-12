package app.wonton.ui

import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** 桥只交给输入框，选房间和发送仍是主人的操作。 */
class TabDraft {
    var text by mutableStateOf<String?>(null)
}

val LocalTabDraft = compositionLocalOf<TabDraft> { error("TabDraft is not provided") }
