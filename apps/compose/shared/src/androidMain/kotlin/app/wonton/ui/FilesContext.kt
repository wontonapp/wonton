package app.wonton.ui

import androidx.compose.runtime.compositionLocalOf
import app.wonton.protocol.Action
import app.wonton.protocol.TransfersViewModel

/** 房间页面给文件卡片的东西。卡片在时间线很深的地方，不一层层传。 */
class FilesContext(val transfers: TransfersViewModel, val onAction: (Action) -> Unit, val onReject: (Problem) -> Unit)

val LocalFiles = compositionLocalOf<FilesContext> { error("房间页面没有提供 FilesContext") }
