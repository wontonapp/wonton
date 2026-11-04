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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import app.wonton.protocol.Action
import app.wonton.protocol.DeviceVerification
import app.wonton.protocol.EncryptionStatus
import app.wonton.protocol.RecoveryStatus
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 聊天记录备份。协议里叫恢复（密钥备份加秘密存储），这里按它对主人的用处说：换设备后还能看到以前的消息。 */
@Composable
fun ChatBackupScreen(encryption: EncryptionStatus, onAction: (Action) -> Unit, onClose: () -> Unit) {
    val colors = WontonTheme.colors
    WontonScreen(bar = { WontonPushedBar("聊天记录备份", onClose) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
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
        }
    }
}

@Composable
private fun NextStep(encryption: EncryptionStatus, onAction: (Action) -> Unit) {
    var key by rememberSaveable { mutableStateOf("") }
    when (encryption.recovery) {
        RecoveryStatus.DISABLED -> if (encryption.verification != DeviceVerification.UNVERIFIED) {
            WontonButton("开启备份", { onAction(Action.SetUpRecovery) }, Modifier.fillMaxWidth())
        }
        RecoveryStatus.INCOMPLETE -> Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
            WontonTextField(
                value = key,
                onValueChange = { key = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = "恢复密钥",
                visualTransformation = PasswordVisualTransformation(),
                // 按密码输入：输入法不联想、不自动改写和大写，否则恢复密钥会被“纠正”成别的字。
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Password,
                ),
            )
            WontonButton(
                "取回以前的消息",
                {
                    onAction(Action.RecoverEncryption(key))
                    key = ""
                },
                Modifier.fillMaxWidth(),
                enabled = key.isNotBlank(),
            )
        }
        RecoveryStatus.IN_PROGRESS -> CircularProgressIndicator()
        RecoveryStatus.ENABLED, RecoveryStatus.UNKNOWN -> Unit
    }
}

val RecoveryStatus.backupTitle: String
    get() = when (this) {
        RecoveryStatus.ENABLED -> "聊天记录已备份"
        RecoveryStatus.DISABLED -> "还没有备份聊天记录"
        RecoveryStatus.INCOMPLETE -> "这台设备还看不到以前的消息"
        RecoveryStatus.IN_PROGRESS -> "正在处理…"
        RecoveryStatus.UNKNOWN -> "还不知道备份的情况"
    }

val EncryptionStatus.backupDetail: String?
    get() = when (recovery) {
        RecoveryStatus.ENABLED -> "换手机或新设备时，输入恢复密钥就能看到以前的消息。"
        // 未验证的设备开启不了，核心会拒绝。
        RecoveryStatus.DISABLED -> if (verification == DeviceVerification.UNVERIFIED) {
            "要到最早配对的那台设备上开启。"
        } else {
            "开启后，换手机或新设备时也能看到以前的消息。"
        }
        RecoveryStatus.INCOMPLETE -> "输入开启备份时保存的恢复密钥。"
        RecoveryStatus.IN_PROGRESS -> null
        RecoveryStatus.UNKNOWN -> "连上家里的服务器后才能确认。"
    }

val RecoveryStatus.backupValue: String
    get() = when (this) {
        RecoveryStatus.ENABLED -> "已开启"
        RecoveryStatus.DISABLED -> "未开启"
        RecoveryStatus.INCOMPLETE -> "需要恢复密钥"
        RecoveryStatus.IN_PROGRESS -> "处理中"
        RecoveryStatus.UNKNOWN -> "不确定"
    }

/** 真需要主人动手时才有，显示在聊天列表顶上。 */
val EncryptionStatus.backupReminder: String?
    get() = when {
        recoveryKey != null -> "恢复密钥还没保存"
        recovery == RecoveryStatus.INCOMPLETE -> "输入恢复密钥，取回以前的消息"
        else -> null
    }

@Composable
private fun RecoveryKey(key: String, onConfirm: () -> Unit) {
    val context = LocalContext.current
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text("保存恢复密钥", style = WontonTheme.typography.itemTitle, color = colors.textPrimary.toColor())
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
            "换手机时要用它。只显示这一次，点“已保存”后本机不再留着它。",
            style = WontonTheme.typography.supporting,
            color = colors.textSecondary.toColor(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonButton("复制", { copySensitive(context, key) }, Modifier.weight(1f), WontonButtonKind.SECONDARY)
            WontonButton("已保存", onConfirm, Modifier.weight(1f))
        }
    }
}

/** 标为敏感内容后，Android 13 起系统的剪贴板提示不显示原文；12L 及以前按官方文档用同名的字符串键。 */
private fun copySensitive(context: Context, text: String) {
    val clip = ClipData.newPlainText("恢复密钥", text)
    val extras = PersistableBundle()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        extras.putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
    } else {
        extras.putBoolean("android.content.extra.IS_SENSITIVE", true)
    }
    clip.description.extras = extras
    context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
}
