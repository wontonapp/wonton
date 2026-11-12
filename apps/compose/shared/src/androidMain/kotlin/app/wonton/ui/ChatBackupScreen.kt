package app.wonton.ui

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import app.wonton.protocol.Action
import app.wonton.protocol.BackupReminder
import app.wonton.protocol.DeviceVerification
import app.wonton.protocol.EncryptionStatus
import app.wonton.protocol.RecoveryStatus
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 聊天记录备份。协议里叫恢复（密钥备份加秘密存储），这里按它对主人的用处说：换设备后还能看到以前的消息。 */
@Composable
fun ChatBackupScreen(encryption: EncryptionStatus, onAction: (Action) -> Unit, onClose: () -> Unit) {
    val colors = WontonTheme.colors
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.chat_backup_title), onClose) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            encryption.recoveryKey?.let { key ->
                RecoveryKey(key, onConfirm = { onAction(Action.ConfirmRecoveryKeySaved) })
            }
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                Text(
                    encryption.recovery.backupTitle,
                    style = WontonTheme.typography.itemTitle,
                    color = colors.textPrimary.toColor(),
                )
                encryption.backupDetail?.let {
                    Text(it, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
                }
            }
            NextStep(encryption, onAction)
            if (encryption.canReset) ResetBackup(onAction)
        }
    }
}

@Composable
private fun ResetBackup(onAction: (Action) -> Unit) {
    var confirming by rememberSaveable { mutableStateOf(false) }
    WontonButton(
        stringResource(R.string.chat_backup_reset), { confirming = true },
        Modifier.fillMaxWidth(), WontonButtonKind.SECONDARY,
    )
    if (confirming) {
        val dismiss = { confirming = false }
        WontonDialog(
            stringResource(R.string.chat_backup_reset_title),
            stringResource(R.string.chat_backup_reset_message),
            dismiss,
            listOf(
                WontonDialogAction(stringResource(R.string.common_cancel), onClick = dismiss),
                WontonDialogAction(stringResource(R.string.chat_backup_reset_confirm), WontonButtonKind.DANGER) {
                    dismiss()
                    onAction(Action.ResetRecovery)
                },
            ),
        )
    }
}

@Composable
private fun NextStep(encryption: EncryptionStatus, onAction: (Action) -> Unit) {
    var key by rememberSaveable { mutableStateOf("") }
    when (encryption.recovery) {
        RecoveryStatus.INCOMPLETE -> Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
            WontonTextField(
                value = key,
                onValueChange = { key = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.chat_backup_recovery_key),
                visualTransformation = PasswordVisualTransformation(),
                // 按密码输入：输入法不联想、不自动改写和大写，否则恢复密钥会被“纠正”成别的字。
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Password,
                ),
            )
            WontonButton(
                stringResource(R.string.chat_backup_recover),
                {
                    onAction(Action.RecoverEncryption(key))
                    key = ""
                },
                Modifier.fillMaxWidth(),
                enabled = key.isNotBlank(),
            )
        }
        RecoveryStatus.IN_PROGRESS -> CircularProgressIndicator()
        RecoveryStatus.DISABLED, RecoveryStatus.ENABLED, RecoveryStatus.UNKNOWN -> Unit
    }
}

val RecoveryStatus.backupTitle: String
    @Composable get() = when (this) {
        RecoveryStatus.ENABLED -> stringResource(R.string.chat_backup_status_enabled)
        RecoveryStatus.DISABLED -> stringResource(R.string.chat_backup_status_disabled)
        RecoveryStatus.INCOMPLETE -> stringResource(R.string.chat_backup_status_incomplete)
        RecoveryStatus.IN_PROGRESS -> stringResource(R.string.chat_backup_status_in_progress)
        RecoveryStatus.UNKNOWN -> stringResource(R.string.chat_backup_status_unknown)
    }

/** 恢复密钥丢了、备份残缺或者开启不了时的出路。正在显示新密钥时不出现，免得主人还没抄下来就把它作废。 */
val EncryptionStatus.canReset: Boolean
    get() = recoveryKey == null && when (recovery) {
        RecoveryStatus.ENABLED, RecoveryStatus.INCOMPLETE -> true
        RecoveryStatus.DISABLED -> verification == DeviceVerification.UNVERIFIED
        RecoveryStatus.IN_PROGRESS, RecoveryStatus.UNKNOWN -> false
    }

val EncryptionStatus.backupDetail: String?
    @Composable get() = when (recovery) {
        RecoveryStatus.ENABLED -> stringResource(R.string.chat_backup_detail_enabled)
        // 未验证的设备开启不了，核心会拒绝。
        RecoveryStatus.DISABLED -> if (verification == DeviceVerification.UNVERIFIED) {
            stringResource(R.string.chat_backup_detail_unverified)
        } else {
            stringResource(R.string.chat_backup_detail_verified)
        }
        RecoveryStatus.INCOMPLETE -> stringResource(R.string.chat_backup_detail_incomplete)
        RecoveryStatus.IN_PROGRESS -> null
        RecoveryStatus.UNKNOWN -> stringResource(R.string.chat_backup_detail_unknown)
    }

val RecoveryStatus.backupValue: String
    @Composable get() = when (this) {
        RecoveryStatus.ENABLED -> stringResource(R.string.chat_backup_value_enabled)
        RecoveryStatus.DISABLED -> stringResource(R.string.chat_backup_value_disabled)
        RecoveryStatus.INCOMPLETE -> stringResource(R.string.chat_backup_value_incomplete)
        RecoveryStatus.IN_PROGRESS -> stringResource(R.string.chat_backup_value_in_progress)
        RecoveryStatus.UNKNOWN -> stringResource(R.string.chat_backup_value_unknown)
    }

val BackupReminder.text: String?
    @Composable get() = when (this) {
        BackupReminder.SAVE_RECOVERY_KEY -> stringResource(R.string.chat_backup_reminder_save_key)
        BackupReminder.ENTER_RECOVERY_KEY -> stringResource(R.string.chat_backup_reminder_enter_key)
        BackupReminder.OTHER -> null
    }

@Composable
private fun RecoveryKey(key: String, onConfirm: () -> Unit) {
    val context = LocalContext.current
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(
            stringResource(R.string.chat_backup_save_key_title),
            style = WontonTheme.typography.itemTitle,
            color = colors.textPrimary.toColor(),
        )
        // 不能长按选中：系统的拷贝不会标为敏感内容，复制只走“复制”按钮。
        Text(
            key,
            Modifier.fillMaxWidth()
                .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
                .padding(WontonTheme.spacing.s12),
            style = WontonTheme.typography.body.copy(fontFamily = FontFamily.Monospace),
            color = colors.textPrimary.toColor(),
        )
        Text(
            stringResource(R.string.chat_backup_save_key_note),
            style = WontonTheme.typography.supporting,
            color = colors.textSecondary.toColor(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonButton(
                stringResource(R.string.chat_backup_copy), { copySensitive(context, key) },
                Modifier.weight(1f), WontonButtonKind.SECONDARY,
            )
            WontonButton(stringResource(R.string.chat_backup_saved), onConfirm, Modifier.weight(1f))
        }
    }
}

/** 标为敏感内容后，Android 13 起系统的剪贴板提示不显示原文；12L 及以前按官方文档用同名的字符串键。 */
private fun copySensitive(context: Context, text: String) {
    // 剪贴板的标签会出现在系统的剪贴板界面里。
    val clip = ClipData.newPlainText(context.getString(R.string.chat_backup_recovery_key), text)
    val extras = PersistableBundle()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        extras.putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
    } else {
        extras.putBoolean("android.content.extra.IS_SENSITIVE", true)
    }
    clip.description.extras = extras
    context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
}
