package app.wonton.ui

import android.content.res.Resources
import app.wonton.protocol.MessageSummary

/** 消息摘要的一行文字：正文原样，图片、文件等写成占位。聊天列表的预览和系统通知共用。 */
fun MessageSummary.line(resources: Resources): String = when (this) {
    is MessageSummary.Text -> text
    is MessageSummary.Files -> resources.getString(R.string.message_summary_files)
    is MessageSummary.Image -> resources.getString(R.string.message_summary_image)
    is MessageSummary.Undecryptable -> resources.getString(R.string.message_summary_undecryptable)
    is MessageSummary.Redacted -> resources.getString(R.string.message_summary_redacted)
    // 事件类型原文（如 m.sticker），不翻译。
    is MessageSummary.Other -> "[$kind]"
}
