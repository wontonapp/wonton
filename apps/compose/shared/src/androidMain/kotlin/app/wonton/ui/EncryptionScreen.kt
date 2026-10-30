package app.wonton.ui

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
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
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.DeviceVerification
import app.wonton.protocol.EncryptionStatus
import app.wonton.protocol.RecoveryStatus

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EncryptionScreen(encryption: EncryptionStatus, onAction: (Action) -> Unit, onClose: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("加密") },
                navigationIcon = { TextButton(onClick = onClose) { Text("完成") } },
            )
        },
    ) { padding ->
        Column(
            Modifier.padding(padding).fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            encryption.recoveryKey?.let { key ->
                RecoveryKey(key, onConfirm = { onAction(Action.ConfirmRecoveryKeySaved) })
            }
            Text("本设备：${verificationText(encryption.verification)}")
            Text("恢复：${recoveryText(encryption.recovery)}")
            Recovery(encryption.recovery, onAction)
        }
    }
}

@Composable
private fun Recovery(recovery: RecoveryStatus, onAction: (Action) -> Unit) {
    var key by rememberSaveable { mutableStateOf("") }
    when (recovery) {
        RecoveryStatus.DISABLED -> Button(onClick = { onAction(Action.SetUpRecovery) }) { Text("设置恢复") }
        RecoveryStatus.INCOMPLETE -> {
            OutlinedTextField(
                value = key,
                onValueChange = { key = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("输入恢复密钥") },
                visualTransformation = PasswordVisualTransformation(),
                // 按密码输入：输入法不联想、不自动改写和大写，否则恢复密钥会被“纠正”成别的字。
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Password,
                ),
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
            )
            Button(
                onClick = {
                    onAction(Action.RecoverEncryption(key))
                    key = ""
                },
                enabled = key.isNotBlank(),
            ) { Text("恢复") }
        }
        RecoveryStatus.IN_PROGRESS -> CircularProgressIndicator()
        RecoveryStatus.ENABLED, RecoveryStatus.UNKNOWN -> Unit
    }
}

@Composable
private fun RecoveryKey(key: String, onConfirm: () -> Unit) {
    val context = LocalContext.current
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("恢复密钥", style = MaterialTheme.typography.titleMedium)
        // 不能长按选中：系统的拷贝不会标为敏感内容，复制只走“复制”按钮。
        Text(key, fontFamily = FontFamily.Monospace)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = { copySensitive(context, key) }) { Text("复制") }
            Button(onClick = onConfirm) { Text("已保存") }
        }
        Text("只显示这一次。点“已保存”后本机不再留着它。", style = MaterialTheme.typography.bodySmall)
        HorizontalDivider()
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

private fun verificationText(verification: DeviceVerification) =
    when (verification) {
        DeviceVerification.VERIFIED -> "已验证"
        DeviceVerification.UNVERIFIED -> "未验证"
        DeviceVerification.UNKNOWN -> "未知"
    }

private fun recoveryText(recovery: RecoveryStatus) =
    when (recovery) {
        RecoveryStatus.ENABLED -> "已开启"
        RecoveryStatus.DISABLED -> "未设置"
        RecoveryStatus.INCOMPLETE -> "缺少密钥"
        RecoveryStatus.IN_PROGRESS -> "进行中"
        RecoveryStatus.UNKNOWN -> "未知"
    }
