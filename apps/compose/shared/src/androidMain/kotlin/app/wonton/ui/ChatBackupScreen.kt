package app.wonton.ui

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import app.wonton.protocol.Action
import app.wonton.protocol.BackupReminder
import app.wonton.protocol.DeviceVerification
import app.wonton.protocol.EncryptionStatus
import app.wonton.protocol.KeyNotice
import app.wonton.protocol.RecoveryNextStep
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

/**
 * 聊天记录备份。协议里叫恢复（密钥备份加秘密存储），这里按它对主人的用处说：换设备后还能看到以前的消息。
 * [onContinueHandoff] 重新打开收起了的交接页（ADR-0087）。
 */
@Composable
fun ChatBackupScreen(
    encryption: EncryptionStatus,
    onAction: (Action) -> Unit,
    onContinueHandoff: () -> Unit,
    onClose: () -> Unit,
) {
    val colors = WontonTheme.colors
    var key by rememberSaveable { mutableStateOf("") }
    var showKeyEntry by rememberSaveable { mutableStateOf(false) }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.chat_backup_title), onClose) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            encryption.recoveryKey?.let { saved ->
                RecoveryKey(saved, onConfirm = { onAction(Action.ConfirmRecoveryKeySaved) })
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
            // 确认流程在这一屏显示，不管恢复是什么状态：这一页开着时弹窗压着不弹（[DeviceVerificationDialogs]），
            // 页面就是唯一看得到请求的地方。流程结束时留着结果，收起弹窗后还在这里看得到。
            encryption.verificationFlow?.let { flow ->
                VerificationFlowContent(flow, onAction, onUseRecoveryKey = { showKeyEntry = true })
            }
            NextStep(encryption, key, { key = it }, showKeyEntry, { showKeyEntry = true }, onAction, onContinueHandoff)
            if (encryption.canReset) ResetBackup(onAction)
        }
    }
}

/**
 * 新设备进来第一件事是输恢复密钥；重置只是丢了钥匙时的退路，做成不抢眼的文字按钮放在最下面。
 */
@Composable
private fun ResetBackup(onAction: (Action) -> Unit) {
    var confirming by rememberSaveable { mutableStateOf(false) }
    val interaction = remember { MutableInteractionSource() }
    Text(
        stringResource(R.string.chat_backup_reset),
        Modifier.fillMaxWidth()
            .clickable(interaction, indication = null, role = Role.Button) { confirming = true }
            .heightIn(min = WontonTheme.size.target)
            .padding(vertical = WontonTheme.spacing.s8),
        style = WontonTheme.typography.supporting,
        color = WontonTheme.colors.textSecondary.toColor(),
        textAlign = TextAlign.Center,
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
private fun NextStep(
    encryption: EncryptionStatus,
    key: String,
    onKey: (String) -> Unit,
    showKeyEntry: Boolean,
    onUseRecoveryKey: () -> Unit,
    onAction: (Action) -> Unit,
    onContinueHandoff: () -> Unit,
) {
    // 类别由核心按恢复状态算好（ADR-0076），这里只按类别画。
    when (encryption.recoveryNextStep) {
        RecoveryNextStep.ENTER_KEY -> RecoveryKeyEntry(key, onKey, onAction)
        // 账号里有别的已验证设备：确认和恢复密钥两条路并列（ADR-0046）。
        RecoveryNextStep.CHOOSE_DEVICE_OR_KEY -> if (showKeyEntry) {
            RecoveryKeyEntry(key, onKey, onAction)
        } else {
            VerificationChoices(onAction, onUseRecoveryKey)
        }
        // 设备确认是这一步：流程在上面那一屏；还没开始时给入口按钮（账号没有备份，输恢复密钥没用）。
        RecoveryNextStep.SHOW_FLOW -> if (encryption.verificationFlow == null) {
            WontonButton(
                stringResource(R.string.verification_verify_with_other_device),
                { onAction(Action.StartDeviceVerification) },
                Modifier.fillMaxWidth(),
            )
        } else if (showKeyEntry) {
            RecoveryKeyEntry(key, onKey, onAction)
        }
        RecoveryNextStep.PROGRESS -> CircularProgressIndicator()
        RecoveryNextStep.WAIT_FOR_OTHER_DEVICE -> WontonButton(
            stringResource(R.string.handoff_title),
            onContinueHandoff,
            Modifier.fillMaxWidth(),
        )
        RecoveryNextStep.NONE, RecoveryNextStep.OTHER -> Unit
    }
}

/** 账号里有别的已验证设备：确认和恢复密钥两条路并列（ADR-0046）。 */
@Composable
private fun VerificationChoices(onAction: (Action) -> Unit, onUseRecoveryKey: () -> Unit) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        WontonButton(
            stringResource(R.string.verification_verify_with_other_device),
            { onAction(Action.StartDeviceVerification) },
            Modifier.weight(1f),
        )
        WontonButton(
            stringResource(R.string.verification_use_recovery_key),
            onUseRecoveryKey,
            Modifier.weight(1f),
            WontonButtonKind.SECONDARY,
        )
    }
}

@Composable
private fun RecoveryKeyEntry(key: String, onKeyChange: (String) -> Unit, onAction: (Action) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        WontonTextField(
            value = key,
            onValueChange = onKeyChange,
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
                onKeyChange("")
            },
            Modifier.fillMaxWidth(),
            enabled = key.isNotBlank(),
        )
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

/** 按钮上的字；`OTHER` 是更新的核心给的新类别，只显示说明。 */
val KeyNotice.action: Int?
    get() = when (this) {
        KeyNotice.ENTER_RECOVERY_KEY -> R.string.verification_use_recovery_key
        KeyNotice.VERIFY_WITH_OTHER_DEVICE -> R.string.verification_verify_with_other_device
        KeyNotice.OTHER -> null
    }

val BackupReminder.text: String?
    @Composable get() = when (this) {
        BackupReminder.SAVE_RECOVERY_KEY -> stringResource(R.string.chat_backup_reminder_save_key)
        BackupReminder.ENTER_RECOVERY_KEY -> stringResource(R.string.chat_backup_reminder_enter_key)
        BackupReminder.CONTINUE_ON_OTHER_DEVICE -> stringResource(R.string.handoff_title)
        BackupReminder.OTHER -> null
    }

@Composable
private fun RecoveryKey(key: String, onConfirm: () -> Unit) {
    val context = LocalContext.current
    val colors = WontonTheme.colors
    // 点“已保存”先确认一次：关掉后本机不再显示这把密钥，误点等于永久丢失（盲测 11-12）。
    var confirming by rememberSaveable { mutableStateOf(false) }
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
            WontonButton(stringResource(R.string.chat_backup_saved), { confirming = true }, Modifier.weight(1f))
        }
    }
    if (confirming) {
        val dismiss = { confirming = false }
        WontonDialog(
            stringResource(R.string.chat_backup_saved_confirm_title),
            stringResource(R.string.chat_backup_saved_confirm_message),
            dismiss,
            listOf(
                WontonDialogAction(stringResource(R.string.common_cancel), onClick = dismiss),
                WontonDialogAction(stringResource(R.string.chat_backup_saved)) {
                    dismiss()
                    onConfirm()
                },
            ),
        )
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
