package app.wonton.ui

import android.app.NotificationManager
import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.wonton.protocol.ChatFailure
import app.wonton.protocol.NotificationCategory
import app.wonton.protocol.PushStatus
import app.wonton.protocol.SettingsEntry
import app.wonton.protocol.SettingsViewModel
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme

val NotificationCategory.notificationChannelId: String
    get() = when (this) {
        NotificationCategory.MESSAGE -> "message"
        NotificationCategory.CONFIRMATION -> "confirmation"
        NotificationCategory.PLUGIN_NOTICE -> "plugin_notice"
    }

val NotificationCategory.notificationTitle: String
    get() = when (this) {
        NotificationCategory.MESSAGE -> "聊天消息"
        NotificationCategory.CONFIRMATION -> "等你确认"
        NotificationCategory.PLUGIN_NOTICE -> "应用通知"
    }

internal val PushStatus.notificationValue: String
    get() = when (this) {
        PushStatus.Registered -> "已开启"
        PushStatus.Registering -> "登记中"
        PushStatus.NoEndpoint -> "未开启"
        is PushStatus.Unavailable, is PushStatus.Failed -> "收不到"
    }

@Composable
internal fun notificationsEnabled(): Boolean {
    val context = LocalContext.current
    var enabled by remember(context) {
        mutableStateOf(NotificationManagerCompat.from(context).areNotificationsEnabled())
    }
    LifecycleResumeEffect(context) {
        enabled = NotificationManagerCompat.from(context).areNotificationsEnabled()
        onPauseOrDispose {}
    }
    return enabled
}

@Composable
fun NotificationSettingsScreen(settings: SettingsViewModel, onBack: () -> Unit) {
    val entry = settings.sections.firstNotNullOf { section ->
        section.entries.firstOrNull { it is SettingsEntry.Notifications } as? SettingsEntry.Notifications
    }
    val context = LocalContext.current
    val enabled = notificationsEnabled()
    var refresh by remember { mutableStateOf(0) }
    LifecycleResumeEffect(context) {
        refresh++
        onPauseOrDispose {}
    }
    val manager = context.getSystemService(NotificationManager::class.java)
    WontonScreen(bar = { WontonPushedBar("通知", onBack) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            WontonGroup {
                WontonRow("系统通知", if (enabled) "已开启" else "已关闭", showsForward = true) {
                    context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
                }
                WontonSeparator()
                WontonRow("推送", entry.push.notificationStatus)
            }
            val detail = when (val push = entry.push) {
                is PushStatus.Failed -> push.message
                is PushStatus.Unavailable -> push.message
                else -> null
            }
            detail?.let { ProblemDetail(it) }
            WontonGroup {
                entry.categories.forEachIndexed { index, category ->
                    if (index > 0) WontonSeparator()
                    val channel = remember(manager, category, refresh) {
                        manager.getNotificationChannel(category.notificationChannelId)
                    }
                    val status = when {
                        !enabled || channel?.importance == NotificationManager.IMPORTANCE_NONE -> "已关闭"
                        channel == null -> "未创建"
                        else -> "已开启"
                    }
                    WontonRow(category.notificationTitle, status, showsForward = true) {
                        context.startActivity(Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                            .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                            .putExtra(Settings.EXTRA_CHANNEL_ID, category.notificationChannelId))
                    }
                }
            }
        }
    }
}

private val PushStatus.notificationStatus: String
    get() = when (this) {
        PushStatus.NoEndpoint -> "未登记"
        PushStatus.Registering -> "正在登记"
        PushStatus.Registered -> "已登记"
        is PushStatus.Unavailable -> "收不到推送，请检查系统通知和分发器"
        is PushStatus.Failed -> when (reason) {
            ChatFailure.TRANSPORT_UNAVAILABLE -> "登记失败，连上家里的服务器后重试"
            ChatFailure.UNAUTHORIZED -> "登记失败，请重新配对"
            ChatFailure.UPSTREAM -> "登记失败，家里的服务器出错了"
            ChatFailure.NOT_READY -> "登记失败，请等聊天连接就绪"
            ChatFailure.INVALID_INPUT -> "登记失败，推送地址无效"
        }
    }
