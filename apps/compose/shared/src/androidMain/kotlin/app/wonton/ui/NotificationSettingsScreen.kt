package app.wonton.ui

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.os.Build
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
import androidx.compose.runtime.saveable.rememberSaveable
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
import app.wonton.protocol.CoreException
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

val NotificationCategory.notificationTitleRes: Int
    get() = when (this) {
        NotificationCategory.MESSAGE -> R.string.notification_channel_message
        NotificationCategory.CONFIRMATION -> R.string.notification_channel_confirmation
        NotificationCategory.PLUGIN_NOTICE -> R.string.notification_channel_plugin
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
    val openSystem = openSystemNotifications(context, enabled)
    val keepalive = entry.method == PushMethod.KEEPALIVE
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.notification_title), onBack) }) {
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
                WontonRow(
                    stringResource(R.string.notification_system),
                    stringResource(
                        if (enabled) R.string.notification_value_on
                        else R.string.notification_value_system_blocked,
                    ),
                    showsForward = true,
                    onClick = openSystem,
                )
                // 推送只在 UnifiedPush 下用：保持连接时新消息由前台服务收，这一行没有意义。
                if (!keepalive) {
                    WontonSeparator()
                    WontonRow(stringResource(R.string.notification_push), entry.push.notificationStatus)
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
                        manager.getNotificationChannel(category.category.notificationChannelId)
                    }
                    val status = when {
                        // 系统没允许时渠道的状态没有意义，先去解决权限。
                        !enabled -> stringResource(R.string.notification_value_system_blocked)
                        channel?.importance == NotificationManager.IMPORTANCE_NONE ->
                            stringResource(R.string.notification_value_off)
                        channel == null -> stringResource(R.string.notification_channel_not_created)
                        else -> stringResource(R.string.notification_value_on)
                    }
                    WontonRow(
                        stringResource(category.category.notificationTitleRes), status, showsForward = true,
                        onClick = if (!enabled) openSystem else {
                            {
                                context.startActivity(Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                                    .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                                    .putExtra(Settings.EXTRA_CHANNEL_ID, category.category.notificationChannelId))
                            }
                        },
                    )
                    WontonSeparator()
                    // 锁屏上给不给看内容（ADR-0101 §5）：默认给，关掉后锁屏只写一句占位。
                    LockScreenToggle(!category.lockScreenPrivate) { shown ->
                        rejection = core.setLockScreen(category.category, !shown)
                    }
                }
            }
        }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

/** 直接调用不返回结果的核心方法（`setLockScreenContent`），失败按同一个 [Problem] 提示。 */
private fun ClientCore.setLockScreen(category: NotificationCategory, hidden: Boolean): Problem? =
    try {
        setLockScreenContent(category, hidden)
        null
    } catch (rejection: CoreException) {
        rejection.problem
    }

/** 一个类别的“锁屏上显示内容”开关。 */
@Composable
private fun LockScreenToggle(shown: Boolean, onChange: (Boolean) -> Unit) {
    val title = stringResource(R.string.notification_lock_screen)
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
            shown, onChange, modifier = Modifier.widthIn(min = WontonTheme.size.target)
                .semantics { contentDescription = title },
        )
    }
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
    @Composable get() = when (this) {
        PushStatus.NoEndpoint -> stringResource(R.string.push_status_no_endpoint)
        PushStatus.Registering -> stringResource(R.string.push_status_registering)
        PushStatus.Registered -> stringResource(R.string.push_status_registered)
        is PushStatus.Unavailable -> stringResource(R.string.push_status_unavailable)
        is PushStatus.Failed -> stringResource(
            when (reason) {
                ChatFailure.TRANSPORT_UNAVAILABLE -> R.string.push_status_failed_transport
                ChatFailure.UNAUTHORIZED -> R.string.push_status_failed_unauthorized
                ChatFailure.FORBIDDEN -> R.string.push_status_failed_forbidden
                ChatFailure.UPSTREAM, ChatFailure.OTHER -> R.string.push_status_failed_upstream
                ChatFailure.NOT_READY -> R.string.push_status_failed_not_ready
                ChatFailure.INVALID_INPUT -> R.string.push_status_failed_invalid_input
            },
        )
    }

/**
 * 系统通知的入口：通知开着时去系统设置管理；没允许时第一次按 Android 规范弹授权框，弹过了
 * （系统对再次请求不再弹框，只会静默拒绝）就带去系统设置开。
 */
@Composable
internal fun openSystemNotifications(context: Context, enabled: Boolean): () -> Unit {
    var asked by rememberSaveable { mutableStateOf(false) }
    val permission = LocalNotificationPermission.current
    val canAsk = permission != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
    return {
        if (enabled || asked || !canAsk) {
            context.startActivity(
                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                    .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName),
            )
        } else {
            permission?.request { granted -> if (!granted) asked = true }
        }
    }
}
