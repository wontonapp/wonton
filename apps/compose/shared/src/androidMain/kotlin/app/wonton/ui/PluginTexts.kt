package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.CardLine
import app.wonton.protocol.PermissionLine
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginTrust
import app.wonton.protocol.Rgb
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.Shield
import app.wonton.ui.icons.WarningFill
import app.wonton.ui.icons.WontonIcon

/** 标签的底是字色的淡化。 */
private const val TAG_ALPHA = 0.14f

/** 正文行高 22、小图标 20，差的一半。 */
private val LINE_ICON_OFFSET = 1.dp

/** 失败以后主人下一步该做什么，跟在核心写的原因后面。 */
val PluginTrust.label: String
    get() = when (this) {
        PluginTrust.OFFICIAL -> "已审核"
        PluginTrust.COMMUNITY, PluginTrust.LOCAL, PluginTrust.UNKNOWN -> "未审核"
        PluginTrust.SYSTEM -> "系统"
    }

val PluginTrust.tint: Rgb
    @Composable get() = when (this) {
        PluginTrust.OFFICIAL -> WontonTheme.colors.success
        PluginTrust.COMMUNITY, PluginTrust.LOCAL, PluginTrust.UNKNOWN -> WontonTheme.colors.warning
        PluginTrust.SYSTEM -> WontonTheme.colors.textSecondary
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
fun Tag(text: String, tint: Rgb) {
    Text(
        text,
        Modifier.background(tint.toColor().copy(alpha = TAG_ALPHA), CircleShape)
            .padding(horizontal = WontonTheme.spacing.s8),
        color = tint.toColor(),
        style = WontonTheme.typography.caption,
    )
}

/** 核心给的说明，`emphasis` 的醒目显示。 */
@Composable
fun NoticeLines(lines: List<CardLine>) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        lines.forEach { line ->
            val tint = (if (line.emphasis) colors.warning else colors.textSecondary).toColor()
            Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                LineIcon(if (line.emphasis) WontonIcon.WarningFill else WontonIcon.Info, tint)
                Text(line.text, style = WontonTheme.typography.body, color = tint)
            }
        }
    }
}

/** 要授予的一项权限：有风险级别时醒目，升级时新申请的、核心不支持的标出来。 */
@Composable
fun PermissionRow(line: PermissionLine) {
    val colors = WontonTheme.colors
    val risky = line.risk != null
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        LineIcon(
            if (risky) WontonIcon.WarningFill else WontonIcon.Shield,
            (if (risky) colors.warning else colors.textSecondary).toColor(),
        )
        Text(
            line.text,
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = (if (risky) colors.warning else colors.textPrimary).toColor(),
        )
        if (line.added) Tag("新增", colors.primary)
        if (!line.supported) Tag("不支持", colors.danger)
    }
}

@Composable
internal fun InstallAccessDetails(notices: List<CardLine>, permissions: List<PermissionLine>) {
    if (notices.isNotEmpty()) NoticeLines(notices)
    if (permissions.isNotEmpty()) {
        DetailSection("权限") { permissions.forEach { PermissionRow(it) } }
    }
}

/** 正文行前面的小图标，和第一行文字对齐。 */
@Composable
fun LineIcon(icon: ImageVector, tint: Color) {
    Icon(
        icon,
        contentDescription = null,
        modifier = Modifier.padding(top = LINE_ICON_OFFSET).size(WontonTheme.size.iconSmall),
        tint = tint,
    )
}

/** 详情和确认页里的一节。 */
@Composable
fun DetailSection(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(
            title,
            Modifier.semantics { heading() },
            style = WontonTheme.typography.sectionTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        content()
    }
}

/** 商店里一段的标题。 */
@Composable
fun StoreSectionTitle(title: String) {
    Text(
        title,
        Modifier.padding(
            start = WontonTheme.spacing.pageInset,
            top = WontonTheme.spacing.s16,
            bottom = WontonTheme.spacing.s4,
        ).semantics { heading() },
        style = WontonTheme.typography.sectionTitle,
        color = WontonTheme.colors.textPrimary.toColor(),
    )
}
