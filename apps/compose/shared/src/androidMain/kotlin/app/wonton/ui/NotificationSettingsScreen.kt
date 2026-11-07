package app.wonton.ui

import android.app.NotificationManager
import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatFailure
import app.wonton.protocol.NotificationCategory
import app.wonton.protocol.PushMethod
import app.wonton.protocol.PushStatus
import app.wonton.protocol.SettingsEntry
import app.wonton.protocol.SettingsViewModel
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

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

/** 设置页“通知”条目里存的收新消息的方式（ADR-0003）。通知条目一直在，取不到说明协议对不上。 */
val SettingsViewModel.pushMethod: PushMethod
    get() = sections
        .firstNotNullOf { section ->
            section.entries.firstOrNull { it is SettingsEntry.Notifications } as? SettingsEntry.Notifications
        }
        .method

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
fun NotificationSettingsScreen(core: ClientCore, settings: SettingsViewModel, onBack: () -> Unit) {
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
    var rejection by remember { mutableStateOf<Problem?>(null) }
    val keepalive = entry.method == PushMethod.KEEPALIVE
    WontonScreen(bar = { WontonPushedBar("通知", onBack) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            WontonGroup {
                KeepaliveToggle(keepalive) { wanted ->
                    val method = if (wanted) PushMethod.KEEPALIVE else PushMethod.UNIFIED_PUSH
                    rejection = core.send(Action.SetPushMethod(method))
                }
                WontonSeparator()
                WontonRow("系统通知", if (enabled) "已开启" else "已关闭", showsForward = true) {
                    context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
                }
                // 推送只在 UnifiedPush 下用：保持连接时新消息由前台服务收，这一行没有意义。
                if (!keepalive) {
                    WontonSeparator()
                    WontonRow("推送", entry.push.notificationStatus)
                }
            }
            if (keepalive && !enabled) {
                Text(
                    stringResource(R.string.keepalive_without_permission),
                    style = WontonTheme.typography.supporting,
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
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
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

/** “保持连接”的开关：开 = 核心收集后台新消息、前台服务保持隧道；关 = 走 UnifiedPush（ADR-0003）。 */
@Composable
private fun KeepaliveToggle(keepalive: Boolean, onChange: (Boolean) -> Unit) {
    val title = stringResource(R.string.keepalive_toggle)
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(
            title, Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        WontonSwitch(
            keepalive, onChange, modifier = Modifier.widthIn(min = WontonTheme.size.target)
                .semantics { contentDescription = title },
        )
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
