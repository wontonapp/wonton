package app.wonton.ui

import android.content.Context
import app.wonton.protocol.BackupReminder

/**
 * 聊天列表顶部“恢复密钥”横幅关掉后的记忆：同类提醒关了就不再挂，设置里的“聊天记录备份”还在。
 * 只关这条横幅，不动核心的备份状态；核心换一类提醒（保存↔输入）时照常出现。
 */
internal class BackupReminderPrefs(context: Context) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun dismissed(reminder: BackupReminder): Boolean =
        prefs.getStringSet(DISMISS_KEY, emptySet()).orEmpty().contains(reminder.name)

    fun dismiss(reminder: BackupReminder) {
        val dismissed = prefs.getStringSet(DISMISS_KEY, emptySet()).orEmpty() + reminder.name
        prefs.edit().putStringSet(DISMISS_KEY, dismissed).apply()
    }

    private companion object {
        const val PREFS = "backup_reminder"
        const val DISMISS_KEY = "dismissed"
    }
}
