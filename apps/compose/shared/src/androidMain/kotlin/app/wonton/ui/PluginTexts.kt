package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.dp
import app.wonton.protocol.CardLine
import app.wonton.protocol.PermissionLine
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginTrust
import app.wonton.protocol.Rgb
import app.wonton.protocol.StoreFailure
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.Shield
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WarningFill
import app.wonton.ui.icons.WontonIcon

/** 标签的底是字色的淡化。 */
private const val TAG_ALPHA = 0.14f

/** 正文行高 22、小图标 20，差的一半。 */
private val LINE_ICON_OFFSET = 1.dp

/** 失败以后主人下一步该做什么，跟在核心写的原因后面。 */
val StoreFailure.text: String
    get() = when (this) {
        StoreFailure.UNREACHABLE -> "连不上 GitHub，列表可能不是最新的"
        StoreFailure.TIMEOUT -> "GitHub 太慢，没有及时答完，列表可能不是最新的"
        StoreFailure.RATE_LIMITED -> "GitHub 限速了，过几分钟再试"
        StoreFailure.INVALID_INDEX -> "商店的列表读不懂，稍后再试"
        StoreFailure.INDEX_TOO_NEW -> "商店的列表比家里的服务器新，先升级家里的服务器"
        StoreFailure.UPSTREAM -> "商店的来源出了问题，稍后再试"
        StoreFailure.INCOMPLETE -> "有的没读到，列表不完整"
        StoreFailure.DISABLED -> "社区应用已关闭"
        StoreFailure.OTHER -> "商店出了问题，稍后再试"
    }

val PluginFailure.nextStep: String
    get() = when (this) {
        PluginFailure.TRANSPORT_UNAVAILABLE -> "连上家里的服务器后再试。"
        PluginFailure.UNAUTHORIZED -> "家里的服务器不认这台设备，请重新扫码配对。"
        PluginFailure.SOURCE_UNAVAILABLE -> "家里的服务器连不上应用的来源，稍后再试。"
        PluginFailure.NOT_FOUND -> "来源里没有这个应用或版本，刷新商店后再试。"
        PluginFailure.CHANGED -> "应用在确认之后变了，请重新查看、确认。"
        PluginFailure.UNSUPPORTED -> "家里的服务器不支持这个应用，先升级家里的服务器。"
        PluginFailure.CONFLICT -> "它已经装了，或者正在安装、升级、卸载，等那边做完。"
        PluginFailure.INVALID -> "应用包不合法，请联系应用的作者。"
        PluginFailure.LAN_UNAVAILABLE -> "在局域网设置里选好网卡或地址后再试。"
        PluginFailure.CORE_UPGRADING -> "家里的服务器正在升级，升级结束后再试。"
        PluginFailure.EGRESS_PROXY_UNAVAILABLE -> "检查家里服务器上的代理，或者在出网代理设置里改地址后再试。"
        PluginFailure.OWNER_PROFILE_INCOMPLETE -> "在设置里填好你的资料后再试。"
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

val PluginTrust.tint: Rgb
    @Composable get() = when (this) {
        PluginTrust.OFFICIAL -> WontonTheme.colors.success
        PluginTrust.COMMUNITY, PluginTrust.UNKNOWN -> WontonTheme.colors.warning
        PluginTrust.LOCAL, PluginTrust.SYSTEM -> WontonTheme.colors.textSecondary
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

/** 取列表、详情或预览失败，整页显示原因和下一步。 */
@Composable
fun ProblemView(message: String, nextStep: String?, onRetry: () -> Unit) {
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            WontonIcon.Warning,
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.icon),
            tint = colors.danger.toColor(),
        )
        Text(
            message,
            style = WontonTheme.typography.itemTitle.copy(textAlign = TextAlign.Center),
            color = colors.textPrimary.toColor(),
        )
        nextStep?.let {
            Text(
                it,
                style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.Center),
                color = colors.textSecondary.toColor(),
            )
        }
        WontonButton("重试", onRetry, Modifier.padding(top = WontonTheme.spacing.s8), WontonButtonKind.SECONDARY)
    }
}

/** 列表还在，但最近一次刷新出了问题：放在列表上方。 */
@Composable
fun ProblemRow(message: String, nextStep: String?) {
    WontonNotice(WontonIcon.Warning, message, lines = listOfNotNull(nextStep))
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

/** 要授予的一项权限：高权限醒目，升级时新申请的、核心不支持的标出来。 */
@Composable
fun PermissionRow(line: PermissionLine) {
    val colors = WontonTheme.colors
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        LineIcon(
            if (line.high) WontonIcon.WarningFill else WontonIcon.Shield,
            (if (line.high) colors.warning else colors.textSecondary).toColor(),
        )
        Text(
            line.text,
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = (if (line.high) colors.warning else colors.textPrimary).toColor(),
        )
        if (line.added) Tag("新增", colors.primary)
        if (!line.supported) Tag("不支持", colors.danger)
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
