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
data class Problem(
    /** 核心给的原文（如核心写的原因）。分类问题不用这个字段，用 [titleRes]。 */
    val title: String = "",
    val nextStep: String? = null,
    val detail: String? = null,
    /** 按类别写在壳里的文字的资源；映射发生在取不到 Composable 上下文的地方（send 的 catch、后台协程）。 */
    val titleRes: Int? = null,
    val nextStepRes: Int? = null,
) {
    /** 分类问题的标题；显示时才取资源。 */
    val titleText: String
        @Composable get() = titleRes?.let { stringResource(it) } ?: title

    /** 分类问题的下一步。 */
    val nextStepText: String?
        @Composable get() = nextStepRes?.let { stringResource(it) } ?: nextStep
}

val AppearanceProblem.problem: Problem
    get() = Problem(
        titleRes = R.string.problem_appearance_title,
        nextStepRes = R.string.problem_storage_next_step,
        detail = message,
    )

fun ChatFailure.problem(detail: String): Problem = when (this) {
    ChatFailure.TRANSPORT_UNAVAILABLE -> problem(
        R.string.problem_transport_unavailable_title,
        R.string.problem_transport_unavailable_next_step,
        detail,
    )
    ChatFailure.UNAUTHORIZED -> problem(
        R.string.problem_unauthorized_title,
        R.string.problem_unauthorized_next_step,
        detail,
    )
    ChatFailure.UPSTREAM -> problem(R.string.problem_upstream_title, R.string.problem_upstream_next_step, detail)
    ChatFailure.NOT_READY -> Problem(titleRes = R.string.problem_not_ready_title, nextStep = detail)
    ChatFailure.INVALID_INPUT -> Problem(titleRes = R.string.problem_invalid_input_title, nextStep = detail)
}

val ChatProblem.problem: Problem
    get() = reason.problem(message)

/** 保存出网代理没成功：连不上时不知道到底改了没有，填错了要照着核心说的改。 */
val PluginProblem.saveProblem: Problem
    get() = when (reason) {
        PluginFailure.TRANSPORT_UNAVAILABLE -> Problem(
            titleRes = R.string.problem_plugin_save_uncertain_title,
            nextStepRes = R.string.problem_plugin_save_uncertain_next_step,
            detail = message,
        )
        PluginFailure.INVALID -> Problem(titleRes = R.string.problem_plugin_save_invalid_title, nextStep = message)
        else -> problem
    }

val PluginProblem.problem: Problem
    get() = reason.problem(message)

fun PluginFailure.problem(detail: String?): Problem = when (this) {
    PluginFailure.TRANSPORT_UNAVAILABLE -> problem(
        R.string.problem_transport_unavailable_title,
        R.string.problem_transport_unavailable_next_step,
        detail,
    )
    PluginFailure.UNAUTHORIZED -> problem(
        R.string.problem_unauthorized_title,
        R.string.problem_unauthorized_next_step,
        detail,
    )
    PluginFailure.SOURCE_UNAVAILABLE -> Problem(
        titleRes = R.string.problem_plugin_source_unavailable_title,
        nextStepRes = R.string.problem_plugin_source_unavailable_next_step,
        detail = detail,
    )
    PluginFailure.NOT_FOUND -> Problem(
        titleRes = R.string.problem_plugin_not_found_title,
        nextStepRes = R.string.problem_plugin_not_found_next_step,
        detail = detail,
    )
    PluginFailure.CHANGED -> Problem(
        titleRes = R.string.problem_plugin_changed_title,
        nextStepRes = R.string.problem_plugin_changed_next_step,
        detail = detail,
    )
    PluginFailure.UNSUPPORTED -> Problem(
        titleRes = R.string.problem_plugin_unsupported_title,
        nextStepRes = R.string.problem_upgrade_server_next_step,
        detail = detail,
    )
    PluginFailure.CONFLICT -> Problem(
        titleRes = R.string.problem_plugin_conflict_title,
        nextStepRes = R.string.problem_plugin_conflict_next_step,
        detail = detail,
    )
    PluginFailure.INVALID -> Problem(
        titleRes = R.string.problem_plugin_invalid_title,
        nextStepRes = R.string.problem_plugin_invalid_next_step,
        detail = detail,
    )
    PluginFailure.DEVICE_UNAVAILABLE -> Problem(
        titleRes = R.string.problem_plugin_device_unavailable_title,
        nextStepRes = R.string.problem_plugin_device_unavailable_next_step,
        detail = detail,
    )
    PluginFailure.LAN_UNAVAILABLE -> Problem(
        titleRes = R.string.problem_plugin_lan_unavailable_title,
        nextStepRes = R.string.problem_plugin_lan_unavailable_next_step,
        detail = detail,
    )
    PluginFailure.LAN_PORT_TAKEN -> Problem(
        titleRes = R.string.problem_plugin_lan_port_taken_title,
        nextStepRes = R.string.problem_plugin_lan_port_taken_next_step,
        detail = detail,
    )
    PluginFailure.CORE_UPGRADING -> Problem(
        titleRes = R.string.problem_plugin_core_upgrading_title,
        nextStepRes = R.string.problem_plugin_core_upgrading_next_step,
        detail = detail,
    )
    PluginFailure.EGRESS_PROXY_UNAVAILABLE -> Problem(
        titleRes = R.string.problem_plugin_egress_proxy_unavailable_title,
        nextStepRes = R.string.problem_plugin_egress_proxy_unavailable_next_step,
        detail = detail,
    )
    PluginFailure.RECORD_UNUSABLE -> Problem(
        titleRes = R.string.problem_plugin_record_unusable_title,
        nextStepRes = R.string.problem_plugin_record_unusable_next_step,
        detail = detail,
    )
    PluginFailure.UPSTREAM -> problem(R.string.problem_upstream_title, R.string.problem_upstream_next_step, detail)
    PluginFailure.PROTOCOL -> Problem(
        titleRes = R.string.problem_plugin_protocol_title,
        nextStepRes = R.string.problem_plugin_protocol_next_step,
        detail = detail,
    )
    PluginFailure.SERVER_TOO_OLD -> Problem(
        titleRes = R.string.problem_plugin_server_too_old_title,
        nextStepRes = R.string.problem_upgrade_server_next_step,
        detail = detail,
    )
    // 比这个壳新的类别：先按服务器出错说。
    else -> problem(R.string.problem_upstream_title, R.string.problem_upstream_next_step, detail)
}

val StoreProblem.problem: Problem
    get() = when (reason) {
        StoreFailure.UNREACHABLE -> Problem(titleRes = R.string.problem_store_unreachable_title, detail = details)
        StoreFailure.TIMEOUT -> Problem(titleRes = R.string.problem_store_timeout_title, detail = details)
        StoreFailure.RATE_LIMITED -> Problem(
            titleRes = R.string.problem_store_rate_limited_title,
            nextStepRes = R.string.problem_store_rate_limited_next_step,
            detail = details,
        )
        StoreFailure.INVALID_INDEX -> Problem(
            titleRes = R.string.problem_store_invalid_index_title,
            nextStepRes = R.string.problem_upstream_next_step,
            detail = details,
        )
        StoreFailure.INDEX_TOO_NEW -> Problem(
            titleRes = R.string.problem_store_index_too_new_title,
            nextStepRes = R.string.problem_upgrade_server_next_step,
            detail = details,
        )
        StoreFailure.UPSTREAM -> Problem(
            titleRes = R.string.problem_store_upstream_title,
            nextStepRes = R.string.problem_upstream_next_step,
            detail = details,
        )
        StoreFailure.INCOMPLETE -> Problem(titleRes = R.string.problem_store_incomplete_title, detail = details)
        StoreFailure.DISABLED -> Problem(titleRes = R.string.problem_store_disabled_title, detail = details)
        StoreFailure.OTHER -> Problem(titleRes = R.string.problem_store_other_title, detail = details)
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
            is CoreException.TransportUnavailable -> problem(
                R.string.problem_transport_unavailable_title,
                R.string.problem_transport_unavailable_next_step,
                detail,
            )
            is CoreException.Unauthorized -> problem(
                R.string.problem_unauthorized_title,
                R.string.problem_unauthorized_next_step,
                detail,
            )
            is CoreException.Upstream -> problem(
                R.string.problem_upstream_title,
                R.string.problem_upstream_next_step,
                detail,
            )
            is CoreException.NotReady -> Problem(titleRes = R.string.problem_not_ready_title, nextStep = reason)
            is CoreException.Storage -> Problem(
                titleRes = R.string.problem_storage_title,
                nextStepRes = R.string.problem_storage_next_step,
                detail = detail,
            )
            is CoreException.InvalidInput -> Problem(titleRes = R.string.problem_invalid_input_title, nextStep = reason)
            // 设置没过校验：逐字段的明细由设置页标到控件旁，这里只说发生了什么。
            is CoreException.SettingsInvalid -> Problem(
                titleRes = R.string.problem_invalid_input_title,
                nextStep = reason,
            )
            is CoreException.ServerTooOld -> Problem(
                titleRes = R.string.problem_plugin_server_too_old_title,
                nextStepRes = R.string.problem_upgrade_server_next_step,
                detail = detail,
            )
            // 比这个壳新的类别：先按服务器出错说。
            else -> problem(R.string.problem_upstream_title, R.string.problem_upstream_next_step, detail)
        }
    }

/** 连不上、不认设备、服务器出错这三句在插件、聊天和连接错误里共用。 */
private fun problem(titleRes: Int, nextStepRes: Int, detail: String?) =
    Problem(titleRes = titleRes, nextStepRes = nextStepRes, detail = detail)

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
                // 装饰：标签文字在左边，箭头只表示能展开。
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
fun ProblemView(problem: Problem, onRetry: (() -> Unit)? = null) {
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            WontonIcon.Warning,
            // 装饰：下面就是标题。
            contentDescription = null,
            Modifier.size(WontonTheme.size.icon),
            tint = colors.textSecondary.toColor(),
        )
        Text(
            problem.titleText,
            style = WontonTheme.typography.itemTitle.copy(textAlign = TextAlign.Center),
            color = colors.textPrimary.toColor(),
        )
        problem.nextStepText?.let {
            Text(
                it,
                style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.Center),
                color = colors.textSecondary.toColor(),
            )
        }
        onRetry?.let {
            WontonButton(
                stringResource(R.string.common_retry),
                it,
                Modifier.padding(vertical = WontonTheme.spacing.s8),
                kind = WontonButtonKind.SECONDARY,
            )
        }
        problem.detail?.let { ProblemDetail(it) }
    }
}

/** 页面还能用，只是这件事没做成。 */
@Composable
fun ProblemNotice(problem: Problem, icon: ImageVector = WontonIcon.Warning) {
    WontonNotice(icon, problem.titleText, lines = listOfNotNull(problem.nextStepText)) {
        problem.detail?.let { ProblemDetail(it) }
    }
}

/** 主人点了什么没做成：说清原因就好，点“好”关掉。 */
@Composable
fun ProblemDialog(problem: Problem, onDismiss: () -> Unit) {
    WontonDialog(
        problem.titleText, problem.nextStepText, onDismiss,
        detail = problem.detail?.let { { ProblemDetail(it) } },
    )
}

/** 列表还在，只是少了点什么或者不是最新的：一条轻提示，不抢列表的位置。 */
@Composable
fun ProblemRow(problem: Problem, onRetry: (() -> Unit)? = null) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.supporting
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Icon(
            WontonIcon.Info,
            // 装饰：提示文字在右边。
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconSmall),
            tint = colors.textSecondary.toColor(),
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(problem.titleText, style = style, color = colors.textPrimary.toColor())
            problem.nextStepText?.let { Text(it, style = style, color = colors.textSecondary.toColor()) }
            problem.detail?.let { ProblemDetail(it) }
        }
        onRetry?.let { WontonSmallButton(stringResource(R.string.common_retry), it) }
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
        stringResource(R.string.problem_slow_loading),
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
    val loadingLabel = stringResource(R.string.problem_loading)
    val secondary = WontonTheme.ratio.secondaryContent
    val pulse by rememberInfiniteTransition(label = "skeleton").animateFloat(
        initialValue = 1f,
        targetValue = secondary,
        animationSpec = infiniteRepeatable(tween(PULSE_MS, easing = LinearEasing), RepeatMode.Reverse),
        label = "skeleton",
    )
    val fill = WontonTheme.colors.surfaceSubtle.toColor()
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.pageInset)
            .semantics { contentDescription = loadingLabel },
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
    ) {
        repeat(SKELETON_ROWS) {
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
        SlowHint()
    }
}
