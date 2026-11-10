package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.ChatAccount
import app.wonton.protocol.ChatFailure
import app.wonton.protocol.ChatForm
import app.wonton.protocol.ChatLayout
import app.wonton.protocol.ChatTextScale
import app.wonton.protocol.CodeOverflow
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow

/**
 * 聊天列表的预览：ChatScreen.kt 已经到 detekt 每文件 11 个函数的上限，预览和它的假数据挪到这里，
 * `Notices`、`InviteItem` 为此外提成 internal。
 */

private fun previewInviteRow() = RoomRow(
    roomId = "!design:home.example",
    name = "Design review",
    isDirect = false,
    directUserId = null,
    isEncrypted = true,
    isReadOnly = false,
    membership = RoomMembership.INVITED,
    preview = null,
    unreadCount = 0u,
    layout = ChatLayout(form = ChatForm.BUBBLES, textScale = ChatTextScale.STANDARD, code = CodeOverflow.WRAP),
)

/** 聊天列表顶部的状态提示（聊天列表）。 */
@Preview(name = "聊天提示", locale = "en", widthDp = 390, heightDp = 160)
@Composable
internal fun ChatNoticesPreview() {
    PreviewFrame {
        Notices(viewModel = previewChat(), onOpenBackup = {})
    }
}

/** 聊天列表里的失败提示：说清原因、给下一步、详情默认收起。 */
@Preview(name = "聊天失败提示", locale = "en", widthDp = 390, heightDp = 220)
@Composable
internal fun ChatFailurePreview() {
    PreviewFrame {
        Notices(
            viewModel = previewChat().copy(
                account = ChatAccount.Failed(reason = ChatFailure.UPSTREAM, message = "502 Bad Gateway"),
            ),
            onOpenBackup = {},
        )
    }
}

/** 有人邀请进房间（聊天列表）。 */
@Preview(name = "聊天邀请", locale = "en", widthDp = 390, heightDp = 160)
@Composable
internal fun ChatInvitePreview() {
    PreviewFrame {
        InviteItem(row = previewInviteRow(), onAction = {})
    }
}
