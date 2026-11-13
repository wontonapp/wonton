package app.wonton.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview

@Preview(name = "Wide chat", locale = "en", widthDp = 1024, heightDp = 768)
@Composable
internal fun WideChatPreview() {
    PreviewFrame {
        AdaptiveChatLayout(
            list = {
                Column {
                    ExpandedChatActions({}, {}, {}, {})
                    Notices(previewChat(), {})
                    InviteItem(previewInviteRow(), {})
                }
            },
            content = { ChatDetailEmpty() },
        )
    }
}
