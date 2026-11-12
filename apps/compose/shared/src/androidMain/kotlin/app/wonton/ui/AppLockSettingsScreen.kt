package app.wonton.ui

import android.content.Intent
import android.provider.Settings
import androidx.activity.compose.LocalActivity
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.fragment.app.FragmentActivity
import app.wonton.client.ClientCore
import app.wonton.protocol.AppLockTimeout
import app.wonton.protocol.AppLockViewModel
import app.wonton.protocol.CoreException
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.WontonIcon

/**
 * 设置里的 App 锁（ADR-0102）：开关和离开后多久要求解锁。
 * 设备没有锁屏或指纹时开不了，如实说明并指去系统设置；设备锁后来被关掉时，锁页同样停在那里等。
 */
@Composable
fun AppLockSettingsScreen(core: ClientCore, view: AppLockViewModel, back: () -> Unit) {
    val context = LocalContext.current
    val activity = LocalActivity.current as? FragmentActivity
    val canAuthenticate = remember(activity) { activity?.let { SystemAuth(it).canAuthenticate() } ?: false }
    var rejection by remember { mutableStateOf<Problem?>(null) }
    val change = { enabled: Boolean, timeout: AppLockTimeout ->
        try {
            core.setAppLock(enabled, timeout)
            rejection = null
        } catch (error: CoreException) {
            rejection = error.problem
        }
    }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.applock_title), back) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            WontonGroup {
                ToggleRow(view.enabled, canAuthenticate) { change(it, view.timeout) }
            }
            when {
                view.enabled -> TimeoutChoices(view.timeout) { change(true, it) }
                !canAuthenticate -> NoDeviceLock {
                    context.startActivity(
                        Intent(Settings.ACTION_SECURITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    )
                }
            }
            view.problem?.let { problem ->
                Text(
                    problem,
                    style = WontonTheme.typography.supporting,
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
            }
        }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

@Composable
private fun ToggleRow(enabled: Boolean, canAuthenticate: Boolean, onChange: (Boolean) -> Unit) {
    val title = stringResource(R.string.applock_title)
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title, Modifier.weight(1f),
            style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor(),
        )
        WontonSwitch(
            enabled, onChange, enabled = canAuthenticate || enabled,
            modifier = Modifier.semantics { contentDescription = title },
        )
    }
}

@Composable
private fun TimeoutChoices(selected: AppLockTimeout, onSelect: (AppLockTimeout) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(
            stringResource(R.string.applock_timeout),
            Modifier.padding(horizontal = WontonTheme.spacing.s16),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        WontonGroup {
            TIMEOUTS.forEachIndexed { index, timeout ->
                if (index > 0) WontonSeparator()
                TimeoutRow(timeout, timeout == selected) { onSelect(timeout) }
            }
        }
    }
}

@Composable
private fun TimeoutRow(timeout: AppLockTimeout, isSelected: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .selectable(selected = isSelected, role = Role.RadioButton, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            stringResource(appLockTimeoutLabel(timeout)), Modifier.weight(1f),
            style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor(),
        )
        if (isSelected) {
            // 装饰：选中状态由选项本身朗读。
            Icon(
                WontonIcon.Check, null,
                Modifier.semantics { hideFromAccessibility() },
                tint = WontonTheme.colors.accentInk.toColor(),
            )
        }
    }
}

/** 按设置页里的显示顺序。 */
private val TIMEOUTS = listOf(
    AppLockTimeout.IMMEDIATE,
    AppLockTimeout.ONE_MINUTE,
    AppLockTimeout.FIVE_MINUTES,
    AppLockTimeout.FIFTEEN_MINUTES,
)

/** 没有锁屏时的说明和出口。 */
@Composable
private fun NoDeviceLock(openSettings: () -> Unit) {
    Column(
        Modifier.padding(horizontal = WontonTheme.spacing.s16),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(
            stringResource(R.string.applock_need_device_lock),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        Spacer(Modifier.height(0.dp))
        WontonButton(stringResource(R.string.applock_open_settings), openSettings)
    }
}

@Composable
internal fun appLockTimeoutLabel(timeout: AppLockTimeout): Int = when (timeout) {
    AppLockTimeout.IMMEDIATE -> R.string.applock_timeout_immediate
    AppLockTimeout.ONE_MINUTE -> R.string.applock_timeout_one_minute
    AppLockTimeout.FIVE_MINUTES -> R.string.applock_timeout_five_minutes
    AppLockTimeout.FIFTEEN_MINUTES -> R.string.applock_timeout_fifteen_minutes
    else -> R.string.applock_timeout_immediate
}
