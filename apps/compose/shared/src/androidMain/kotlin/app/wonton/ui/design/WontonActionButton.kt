package app.wonton.ui.design

import android.os.Build
import android.view.HapticFeedbackConstants
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import app.wonton.protocol.ActionFailure
import app.wonton.protocol.ActionRun
import app.wonton.ui.ProblemDetail
import app.wonton.ui.R
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/** 破坏性的动作先确认：写清会怎样，“取消”在前。 */
class WontonActionConfirm(val title: String, val message: String?, val confirm: String)

/**
 * 让核心去做一件事的按钮（ADR-0026 的“动作按钮的反馈”）。按下变淡并轻震一下；[run] 是核心给的进度：
 * 进行中转圈、不能再点，没做成时由同一处的 [WontonActionNotice] 说一句人话。破坏性的给 [confirm]，先确认再做。
 * 壳不自己记点没点过、做没做成，进度只听核心的。
 */
@Composable
fun WontonActionButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: WontonButtonKind = WontonButtonKind.SECONDARY,
    small: Boolean = false,
    run: ActionRun = ActionRun.Idle,
    confirm: WontonActionConfirm? = null,
    testTag: String? = null,
) {
    val view = LocalView.current
    val running = run is ActionRun.Running
    var confirming by remember { mutableStateOf(false) }
    var lastPhase by remember { mutableStateOf(run.phase) }
    val phase = run.phase
    LaunchedEffect(phase) {
        // 只在进度变了时震：页面重组、刚出现时停在“没做成”不震。
        if (phase != lastPhase) phase.haptic?.let(view::performHapticFeedback)
        lastPhase = phase
    }
    val tap = {
        if (!running) {
            view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
            if (confirm != null) confirming = true else onClick()
        }
    }
    val busy = stringResource(R.string.design_action_running)
    val tagged = modifier
        .then(if (testTag != null) Modifier.testTag(testTag) else Modifier)
        .semantics { if (running) stateDescription = busy }
    val ink = when (kind) {
        WontonButtonKind.PRIMARY -> WontonTheme.colors.onPrimary
        WontonButtonKind.SECONDARY -> WontonTheme.colors.textPrimary
        WontonButtonKind.DANGER -> WontonTheme.colors.danger
    }.toColor()
    val spinner: (@Composable () -> Unit)? = if (running) ({ Spinner(ink) }) else null
    if (small) {
        WontonSmallButton(text, tap, enabled = !running, danger = kind == WontonButtonKind.DANGER,
            modifier = tagged, leading = spinner)
    } else {
        WontonButton(text, tap, tagged, kind = kind, enabled = !running, leading = spinner)
    }
    if (confirming && confirm != null) {
        val dismiss = { confirming = false }
        WontonDialog(
            confirm.title, confirm.message, dismiss,
            actions = listOf(
                WontonDialogAction(stringResource(R.string.common_cancel), onClick = dismiss),
                WontonDialogAction(confirm.confirm, WontonButtonKind.DANGER) { confirming = false; onClick() },
            ),
        )
    }
}

@Composable
private fun Spinner(color: Color) {
    CircularProgressIndicator(Modifier.size(WontonTheme.size.iconTiny), color = color, strokeWidth = 2.dp)
}

/** 动作没做成或不确定时的那一句，放在按钮所在的那一组下面；原文收进默认收起的“详情”。别的进度不占地方。 */
@Composable
fun WontonActionNotice(run: ActionRun, modifier: Modifier = Modifier) {
    val (text, tint, details) = when (run) {
        is ActionRun.Failed -> Triple(run.failure.text(), WontonTheme.colors.danger, run.details)
        is ActionRun.Uncertain ->
            Triple(stringResource(R.string.design_action_uncertain), WontonTheme.colors.textSecondary, null)
        else -> return
    }
    Column(
        modifier.testTag("actionNotice"),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            verticalAlignment = Alignment.CenterVertically) {
            Icon(
                if (run is ActionRun.Failed) WontonIcon.Warning else WontonIcon.Info,
                // 装饰：右边就是那句话。
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconTiny),
                tint = tint.toColor(),
            )
            Text(text, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textPrimary.toColor())
        }
        details?.let { ProblemDetail(it, toggleTag = "actionNoticeDetails") }
    }
}

/** 进度的几档：成了、没成、不确定各震一种，进行中和回到空闲不震。 */
enum class ActionPhase(val haptic: Int?) {
    IDLE(null),
    RUNNING(null),
    SUCCEEDED(if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) HapticFeedbackConstants.CONFIRM else null),
    FAILED(
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            HapticFeedbackConstants.REJECT
        } else {
            HapticFeedbackConstants.LONG_PRESS
        },
    ),
    UNCERTAIN(HapticFeedbackConstants.CONTEXT_CLICK),
}

val ActionRun.phase: ActionPhase
    get() = when (this) {
        is ActionRun.Running -> ActionPhase.RUNNING
        is ActionRun.Succeeded -> ActionPhase.SUCCEEDED
        is ActionRun.Failed -> ActionPhase.FAILED
        is ActionRun.Uncertain -> ActionPhase.UNCERTAIN
        is ActionRun.Idle, is ActionRun.Other -> ActionPhase.IDLE
    }

/** 一句人话，按类别写；原文在详情里。 */
@Composable
fun ActionFailure.text(): String = stringResource(
    when (this) {
        ActionFailure.TRANSPORT -> R.string.design_action_failed_transport
        ActionFailure.PERMISSION -> R.string.design_action_failed_permission
        ActionFailure.NOT_READY -> R.string.design_action_failed_not_ready
        ActionFailure.UPSTREAM -> R.string.design_action_failed_upstream
        ActionFailure.STILL_FAILING -> R.string.design_action_failed_still
        ActionFailure.INVALID_INPUT -> R.string.design_action_failed_invalid
        ActionFailure.OTHER -> R.string.design_action_failed_other
    },
)
