package app.wonton.ui

import android.content.res.Resources
import app.wonton.protocol.MessageSummary
import app.wonton.protocol.SideChatState

/** 消息摘要的一行文字：正文原样，图片、文件等写成占位。聊天列表的预览和系统通知共用。 */
fun MessageSummary.line(resources: Resources): String = when (this) {
    is MessageSummary.Text -> text
    is MessageSummary.Files -> resources.getString(R.string.message_summary_files)
    is MessageSummary.Image -> resources.getString(R.string.message_summary_image)
    is MessageSummary.Undecryptable -> resources.getString(R.string.message_summary_undecryptable)
    is MessageSummary.Redacted -> resources.getString(R.string.message_summary_redacted)
    // 旁聊的根、结果行或旁聊里的消息（ADR-0093 §4）：预览给旁聊的标题和状态。不回报结果的 AI 没有状态可说，
    // 那句“结果不会带回主对话”的提示留在卡片和旁聊页上，预览只给标题。
    is MessageSummary.SideChat -> if (state == SideChatState.UNTRACKED) {
        title
    } else {
        resources.getString(R.string.side_chat_result_line, title, resources.getString(state.textRes))
    }
    // 事件类型原文（如 m.sticker），不翻译。
    is MessageSummary.Other -> "[$kind]"
}
