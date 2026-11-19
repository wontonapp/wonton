package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.RoomAgentStop
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 一条时间线的停止控件（ADR-0061 第 10 节）：核心算好的样子。取消和释放同 [chatUpdates]。 */
fun ClientCore.agentStopUpdates(target: TimelineTarget): Flow<RoomAgentStop> = flow {
    subscribeAgentStop(target).use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** 这个状态下要不要画停止钮：没有在跑的、刚停下的都不画。 */
val RoomAgentStop.showsButton: Boolean
    get() = when (this) {
        RoomAgentStop.Running, RoomAgentStop.Stopping, is RoomAgentStop.Failed, is RoomAgentStop.Uncertain -> true
        RoomAgentStop.Idle, RoomAgentStop.Stopped -> false
    }

/**
 * 输入栏里停下 Bot 这一轮的圆钮。核心说有轮次在跑才出现；请求在路上时转圈、点不了，减弱动态效果时不转。
 * 没停成、不确定时按钮还在，再点就是重试，原因在输入栏上方的 [AgentStopNotice]。
 */
@Composable
fun AgentStopButton(state: RoomAgentStop, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val stopping = state == RoomAgentStop.Stopping
    val label = stringResource(if (stopping) R.string.composer_stopping else R.string.composer_stop)
    Box(
        Modifier.size(WontonTheme.size.target)
            .clickable(enabled = !stopping, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label }
            .testTag("composer-stop"),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(WontonTheme.size.sendButton).background(colors.primary.toColor(), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            if (stopping && !AvatarEnvironment.reduceMotion) {
                CircularProgressIndicator(
                    Modifier.size(WontonTheme.size.iconSmall),
                    color = colors.onPrimary.toColor(),
                    strokeWidth = 2.dp,
                )
            } else {
                // 停止的方块是画出来的形状，不是图标资源。
                Box(
                    Modifier.size(WontonTheme.size.iconTiny)
                        .alpha(if (stopping) WontonTheme.ratio.secondaryContent else 1f)
                        .background(colors.onPrimary.toColor(), RoundedCornerShape(2.dp)),
                )
            }
        }
    }
}

/** 输入栏上方的一行：停的结果。停成了说一声，核心过几秒自己收走；没停成、不确定按类别说，原因收在“详情”里。 */
@Composable
fun AgentStopNotice(state: RoomAgentStop) {
    when (state) {
        RoomAgentStop.Stopped -> Note(R.string.composer_stopped, null, "composer-stop-stopped")
        is RoomAgentStop.Failed -> {
            // 身份对、但这一轮不让这个人停：再点也一样，文字不叫人重试。
            val refused = state.error is CoreException.Forbidden || state.error is CoreException.Unauthorized
            Note(
                if (refused) R.string.composer_stop_forbidden else R.string.composer_stop_failed,
                state.error,
                "composer-stop-failed",
            )
        }
        is RoomAgentStop.Uncertain -> Note(R.string.composer_stop_uncertain, state.error, "composer-stop-uncertain")
        RoomAgentStop.Idle, RoomAgentStop.Running, RoomAgentStop.Stopping -> Unit
    }
}

@Composable
private fun Note(text: Int, error: CoreException?, tag: String) {
    val spacing = WontonTheme.spacing
    Column(Modifier.fillMaxWidth().padding(start = spacing.s16, end = spacing.s16, top = spacing.s8).testTag(tag)) {
        Text(
            stringResource(text),
            style = WontonTheme.typography.caption,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        if (error != null) ProblemDetail(error.toString())
    }
}
