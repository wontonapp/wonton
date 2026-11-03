package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.wonton.protocol.CardLine
import app.wonton.protocol.PermissionLine
import app.wonton.protocol.PluginCategory
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginTrust

val Blue = Color(0xFF0A7CFF)
val Orange = Color(0xFFE67E00)
val Gray = Color(0xFF8E8E93)
val Red = Color(0xFFE5342B)

val PluginCategory.label: String
    get() = when (this) {
        PluginCategory.AI -> "AI"
        PluginCategory.AGENT -> "Agent"
        PluginCategory.PHOTOS -> "照片"
        PluginCategory.MEDIA -> "影音"
        PluginCategory.FILES -> "文件"
        PluginCategory.DOCUMENTS -> "文档"
        PluginCategory.PRODUCTIVITY -> "效率"
        PluginCategory.COMMUNICATION -> "通讯"
        PluginCategory.HOME -> "智能家居"
        PluginCategory.NETWORK -> "网络"
        PluginCategory.SECURITY -> "安全"
        PluginCategory.DEVELOPER -> "开发"
        PluginCategory.DOWNLOADS -> "下载"
        PluginCategory.TOOLS -> "工具"
        PluginCategory.SYSTEM -> "系统"
        PluginCategory.OTHER -> "其他"
    }

/** 失败以后主人下一步该做什么，跟在核心写的原因后面。 */
val PluginFailure.nextStep: String
    get() = when (this) {
        PluginFailure.TRANSPORT_UNAVAILABLE -> "连上家里的服务器后再试。"
        PluginFailure.UNAUTHORIZED -> "家里的服务器不认这台设备，请重新扫码配对。"
        PluginFailure.SOURCE_UNAVAILABLE -> "家里的服务器连不上插件的来源，稍后再试。"
        PluginFailure.NOT_FOUND -> "来源里没有这个插件或版本，刷新商店后再试。"
        PluginFailure.CHANGED -> "插件在确认之后变了，请重新查看、确认。"
        PluginFailure.UNSUPPORTED -> "家里的服务器不支持这个插件，先升级家里的服务器。"
        PluginFailure.CONFLICT -> "它已经装了，或者正在安装、升级、卸载，等那边做完。"
        PluginFailure.INVALID -> "插件包不合法，请联系插件的作者。"
        PluginFailure.LAN_UNAVAILABLE -> "在局域网设置里选好网卡或地址后再试。"
        PluginFailure.CORE_UPGRADING -> "家里的服务器正在升级，升级结束后再试。"
        PluginFailure.UPSTREAM -> "家里的服务器出了错，稍后再试。"
        PluginFailure.PROTOCOL -> "App 和家里的服务器版本对不上，请更新 App 或家里的服务器。"
    }

val PluginTrust.label: String
    get() = when (this) {
        PluginTrust.OFFICIAL -> "已审核"
        PluginTrust.COMMUNITY, PluginTrust.UNKNOWN -> "未审核"
        PluginTrust.LOCAL -> "本地"
        PluginTrust.SYSTEM -> "系统"
    }

val PluginTrust.tint: Color
    get() = when (this) {
        PluginTrust.OFFICIAL -> Blue
        PluginTrust.COMMUNITY, PluginTrust.UNKNOWN -> Orange
        PluginTrust.LOCAL, PluginTrust.SYSTEM -> Gray
    }

val PluginOperation.runningText: String
    get() = when (this) {
        PluginOperation.INSTALL -> "正在安装"
        PluginOperation.UPGRADE -> "正在升级"
        PluginOperation.UNINSTALL -> "正在卸载"
    }

val PluginOperation.doneText: String
    get() = when (this) {
        PluginOperation.INSTALL -> "已安装"
        PluginOperation.UPGRADE -> "已升级"
        PluginOperation.UNINSTALL -> "已卸载"
    }

val PluginOperation.failedText: String
    get() = when (this) {
        PluginOperation.INSTALL -> "安装失败"
        PluginOperation.UPGRADE -> "升级失败"
        PluginOperation.UNINSTALL -> "卸载失败"
    }

/** 来源、高权限、新增的权限这类小标签。 */
@Composable
fun Tag(text: String, tint: Color) {
    Text(
        text,
        Modifier.background(tint.copy(alpha = 0.14f), CircleShape)
            .padding(horizontal = 6.dp, vertical = 1.dp),
        color = tint,
        style = MaterialTheme.typography.labelSmall,
        fontWeight = FontWeight.SemiBold,
    )
}

/** 取列表、详情或预览失败，整页显示原因和下一步。 */
@Composable
fun ProblemView(message: String, nextStep: String?, onRetry: () -> Unit) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(message, style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
        nextStep?.let {
            Text(it, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
        }
        OutlinedButton(onClick = onRetry, Modifier.padding(top = 8.dp)) { Text("重试") }
    }
}

/** 列表还在，但最近一次刷新出了问题：放在列表上方。 */
@Composable
fun ProblemRow(message: String, nextStep: String?, modifier: Modifier = Modifier) {
    Column(
        modifier.fillMaxWidth()
            .background(Orange.copy(alpha = 0.12f), RoundedCornerShape(12.dp))
            .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        Text(message, style = MaterialTheme.typography.bodySmall)
        nextStep?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** 核心给的说明，`emphasis` 的醒目显示。 */
@Composable
fun NoticeLines(lines: List<CardLine>) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        lines.forEach { line ->
            Text(
                line.text,
                color = if (line.emphasis) Orange else MaterialTheme.colorScheme.onSurfaceVariant,
                fontWeight = if (line.emphasis) FontWeight.SemiBold else null,
            )
        }
    }
}

/** 要授予的一项权限：高权限醒目，升级时新申请的、核心不支持的标出来。 */
@Composable
fun PermissionRow(line: PermissionLine) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(
            line.text,
            Modifier.weight(1f),
            color = if (line.high) Orange else Color.Unspecified,
            fontWeight = if (line.high) FontWeight.SemiBold else null,
        )
        if (line.added) Tag("新增", Blue)
        if (!line.supported) Tag("不支持", Red)
    }
}
