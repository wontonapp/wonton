package app.wonton.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.wonton.protocol.AppearanceProblem
import app.wonton.protocol.ChatFailure
import app.wonton.protocol.ChatProblem
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginProblem
import app.wonton.protocol.StoreFailure
import app.wonton.protocol.StoreProblem
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.delay

private const val EXPANDED_DEGREES = 90f
private const val SLOW_AFTER_MS = 5_000L
private const val PULSE_MS = 900
private const val SKELETON_ROWS = 5
private val SKELETON_TITLE = 120.dp

/**
 * 给人看的问题：[title] 说发生了什么，[nextStep] 说能做什么，[detail] 是核心写的技术细节，默认收起。类别由核心给，
 * 文字按类别写在这里。
 */
data class Problem(val title: String, val nextStep: String? = null, val detail: String? = null)

val AppearanceProblem.problem: Problem
    get() = Problem("外观设置没有存上", "检查存储空间后再试。", message)

fun ChatFailure.problem(detail: String): Problem = when (this) {
    ChatFailure.TRANSPORT_UNAVAILABLE -> Problem("连不上家里的服务器", "检查网络，连上后再试。", detail)
    ChatFailure.UNAUTHORIZED -> Problem("家里的服务器不认这台设备", "请重新扫码配对。", detail)
    ChatFailure.UPSTREAM -> Problem("家里的服务器出错了", "稍后再试。", detail)
    ChatFailure.NOT_READY -> Problem("现在还做不了", detail)
    ChatFailure.INVALID_INPUT -> Problem("这个操作做不了", detail)
}

val ChatProblem.problem: Problem
    get() = reason.problem(message)

/** 保存出网代理没成功：连不上时不知道到底改了没有，填错了要照着核心说的改。 */
val PluginProblem.saveProblem: Problem
    get() = when (reason) {
        PluginFailure.TRANSPORT_UNAVAILABLE -> Problem("不确定改好了没有", "连上家里的服务器后再看一下。", message)
        PluginFailure.INVALID -> Problem("这样填不对", message)
        else -> problem
    }

val PluginProblem.problem: Problem
    get() = reason.problem(message)

fun PluginFailure.problem(detail: String?): Problem = when (this) {
    PluginFailure.TRANSPORT_UNAVAILABLE -> Problem("连不上家里的服务器", "检查网络，连上后再试。", detail)
    PluginFailure.UNAUTHORIZED -> Problem("家里的服务器不认这台设备", "请重新扫码配对。", detail)
    PluginFailure.SOURCE_UNAVAILABLE -> Problem("暂时取不到应用的信息", "家里的服务器连不上应用的来源，稍后再试。", detail)
    PluginFailure.NOT_FOUND -> Problem("没有找到这个应用", "刷新应用商店后再试。", detail)
    PluginFailure.CHANGED -> Problem("应用在确认之后变了", "请重新查看、确认。", detail)
    PluginFailure.UNSUPPORTED -> Problem("家里的服务器不支持这个应用", "先升级家里的服务器。", detail)
    PluginFailure.CONFLICT -> Problem("这个应用正忙", "它已经装了，或者正在安装、升级、卸载，等那边做完。", detail)
    PluginFailure.INVALID -> Problem("这个应用的安装包有问题", "请联系应用的作者。", detail)
    PluginFailure.DEVICE_UNAVAILABLE -> Problem("选择的设备不可用", "重新查看设备后再试。", detail)
    PluginFailure.LAN_UNAVAILABLE -> Problem("选不出局域网地址", "在局域网设置里选好网卡或地址后再试。", detail)
    PluginFailure.CORE_UPGRADING -> Problem("家里的服务器正在升级", "升级结束后再试。", detail)
    PluginFailure.EGRESS_PROXY_UNAVAILABLE ->
        Problem("代理连不上", "检查家里服务器上的代理，或者在出网代理设置里改地址后再试。", detail)
    PluginFailure.OWNER_PROFILE_INCOMPLETE -> Problem("你的资料还没填好", "在设置里填好后再试。", detail)
    PluginFailure.UPSTREAM -> Problem("家里的服务器出错了", "稍后再试。", detail)
    PluginFailure.PROTOCOL -> Problem("App 和家里的服务器版本对不上", "请更新 App 或家里的服务器。", detail)
}

val StoreProblem.problem: Problem
    get() = when (reason) {
        StoreFailure.UNREACHABLE -> Problem("连不上应用的来源，列表可能不是最新的", detail = details)
        StoreFailure.TIMEOUT -> Problem("应用的来源太慢了，列表可能不是最新的", detail = details)
        StoreFailure.RATE_LIMITED -> Problem("请求太频繁了", "过几分钟再试。", details)
        StoreFailure.INVALID_INDEX -> Problem("应用列表读不懂", "稍后再试。", details)
        StoreFailure.INDEX_TOO_NEW -> Problem("应用列表比家里的服务器新", "先升级家里的服务器。", details)
        StoreFailure.UPSTREAM -> Problem("应用的来源出了问题", "稍后再试。", details)
        StoreFailure.INCOMPLETE -> Problem("有几个应用没加载出来", detail = details)
        StoreFailure.DISABLED -> Problem("社区应用已关闭", detail = details)
        StoreFailure.OTHER -> Problem("应用列表可能不完整", detail = details)
    }

/** 主人自己关掉的，重试也不会变。 */
val StoreProblem.canRetry: Boolean
    get() = reason != StoreFailure.DISABLED

/** 详情页、确认页取内容失败的结果。 */
val Result<*>.failureProblem: Problem
    get() = (exceptionOrNull() as CoreException).problem

val CoreException.problem: Problem
    get() {
        val detail = toString()
        return when (this) {
            is CoreException.TransportUnavailable -> Problem("连不上家里的服务器", "检查网络，连上后再试。", detail)
            is CoreException.Unauthorized -> Problem("家里的服务器不认这台设备", "请重新扫码配对。", detail)
            is CoreException.Upstream -> Problem("家里的服务器出错了", "稍后再试。", detail)
            is CoreException.NotReady -> Problem("现在还做不了", reason)
            is CoreException.Storage -> Problem("这台设备读写数据出错了", "检查存储空间后再试。", detail)
            is CoreException.InvalidInput -> Problem("这个操作做不了", reason)
        }
    }

/** 收起的技术细节。排查时才看，能选中复制。 */
@Composable
fun ProblemDetail(
    text: String,
    label: String = stringResource(R.string.common_details),
    modifier: Modifier = Modifier,
) {
    var expanded by remember { mutableStateOf(false) }
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.caption
    val tint = colors.textSecondary.toColor()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Row(
            modifier.clickable(role = Role.Button) { expanded = !expanded },
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(label, style = style, color = tint)
            Icon(
                WontonIcon.Forward,
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconTiny).rotate(if (expanded) EXPANDED_DEGREES else 0f),
                tint = tint,
            )
        }
        if (expanded) {
            SelectionContainer { Text(text, style = style, color = tint) }
        }
    }
}

/** 什么都显示不出来，整页说明。 */
@Composable
fun ProblemView(problem: Problem, onRetry: () -> Unit) {
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            WontonIcon.Warning,
            contentDescription = null,
            Modifier.size(WontonTheme.size.icon),
            tint = colors.textSecondary.toColor(),
        )
        Text(
            problem.title,
            style = WontonTheme.typography.itemTitle.copy(textAlign = TextAlign.Center),
            color = colors.textPrimary.toColor(),
        )
        problem.nextStep?.let {
            Text(
                it,
                style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.Center),
                color = colors.textSecondary.toColor(),
            )
        }
        WontonButton(
            "重试",
            onRetry,
            Modifier.padding(vertical = WontonTheme.spacing.s8),
            kind = WontonButtonKind.SECONDARY,
        )
        problem.detail?.let { ProblemDetail(it) }
    }
}

/** 页面还能用，只是这件事没做成。 */
@Composable
fun ProblemNotice(problem: Problem, icon: ImageVector = WontonIcon.Warning) {
    WontonNotice(icon, problem.title, lines = listOfNotNull(problem.nextStep)) {
        problem.detail?.let { ProblemDetail(it) }
    }
}

/** 主人点了什么没做成：说清原因就好，点“好”关掉。 */
@Composable
fun ProblemDialog(problem: Problem, onDismiss: () -> Unit) {
    WontonDialog(problem.title, problem.nextStep, onDismiss, detail = problem.detail?.let { { ProblemDetail(it) } })
}

/** 列表还在，只是少了点什么或者不是最新的：一条轻提示，不抢列表的位置。 */
@Composable
fun ProblemRow(problem: Problem, onRetry: (() -> Unit)? = null) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.supporting
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Icon(
            WontonIcon.Info,
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconSmall),
            tint = colors.textSecondary.toColor(),
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(problem.title, style = style, color = colors.textPrimary.toColor())
            problem.nextStep?.let { Text(it, style = style, color = colors.textSecondary.toColor()) }
            problem.detail?.let { ProblemDetail(it) }
        }
        onRetry?.let { WontonSmallButton("重试", it) }
    }
}

/** 过了几秒还没好时才出现的一句话。 */
@Composable
private fun SlowHint() {
    var slow by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        delay(SLOW_AFTER_MS)
        slow = true
    }
    Text(
        "还在加载，网络慢时要多等一会儿",
        Modifier.fillMaxWidth().alpha(if (slow) 1f else 0f),
        style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.Center),
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}

/** 取一个页面的内容：转圈，慢了说一句。 */
@Composable
fun LoadingView() {
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator(color = WontonTheme.colors.textPrimary.toColor())
        SlowHint()
    }
}

/** 取列表：骨架占住列表的位置，慢了说一句。 */
@Composable
fun ListSkeleton(iconSize: Dp) {
    val secondary = WontonTheme.ratio.secondaryContent
    val pulse by rememberInfiniteTransition(label = "skeleton").animateFloat(
        initialValue = 1f,
        targetValue = secondary,
        animationSpec = infiniteRepeatable(tween(PULSE_MS, easing = LinearEasing), RepeatMode.Reverse),
        label = "skeleton",
    )
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.pageInset).semantics { contentDescription = "正在加载" },
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
    ) {
        repeat(SKELETON_ROWS) { SkeletonRow(iconSize, pulse) }
        SlowHint()
    }
}

@Composable
private fun SkeletonRow(iconSize: Dp, pulse: Float) {
    val fill = WontonTheme.colors.surfaceSubtle.toColor()
    Row(
        Modifier.alpha(pulse),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(iconSize).background(fill, pluginIconShape(iconSize)))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            val line = Modifier.height(WontonTheme.spacing.s12).background(fill, CircleShape)
            Box(line.widthIn(max = SKELETON_TITLE).fillMaxWidth())
            Box(line.fillMaxWidth())
        }
    }
}
