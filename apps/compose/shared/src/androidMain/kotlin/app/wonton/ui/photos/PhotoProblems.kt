package app.wonton.ui.photos

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import app.wonton.protocol.MediaException
import app.wonton.protocol.MediaFailure
import app.wonton.protocol.MediaFailureCode
import app.wonton.ui.Problem
import app.wonton.ui.ProblemDetail
import app.wonton.ui.R
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/** 核心给的失败类别写成给人看的一句话和下一步；核心写的原文收进“详情”。认不出的码落到通用的那一句。 */
internal val MediaFailure.problem: Problem
    get() {
        val (title, next) = when (code) {
            MediaFailureCode.TransportUnavailable ->
                R.string.problem_transport_unavailable_title to R.string.problem_transport_unavailable_next_step
            MediaFailureCode.PluginUnavailable ->
                R.string.photo_library_unavailable_title to R.string.photo_library_unavailable_next_step
            // 家里能出相册的应用不止一个、又没定默认：核心不替人挑。能选哪几个的名单核心还不给，这里只说明。
            MediaFailureCode.SourceRequired ->
                R.string.photo_library_source_required_title to R.string.photo_library_source_required_next_step
            MediaFailureCode.PermissionDenied ->
                R.string.photo_library_denied_title to R.string.photo_library_denied_next_step
            MediaFailureCode.AuthenticationRequired ->
                R.string.photo_library_unrecognized_title to R.string.photo_library_try_later
            MediaFailureCode.RateLimited ->
                R.string.photo_library_busy_title to R.string.photo_library_try_later
            MediaFailureCode.MediaGone ->
                R.string.photo_library_gone_title to R.string.photo_library_gone_next_step
            MediaFailureCode.UpstreamError ->
                R.string.photo_library_upstream_title to R.string.photo_library_try_later
            else -> R.string.photo_library_failed_title to R.string.photo_library_try_later
        }
        return Problem(titleRes = title, nextStepRes = next, detail = message)
    }

/** 图片加载器抛出来的原因：核心报的失败照类别说，别的（读回环断了）按连不上说。 */
internal fun Throwable.photoProblem(): Problem = when (this) {
    is MediaException.Failed -> failure.problem
    else -> Problem(
        titleRes = R.string.photo_library_failed_title,
        nextStepRes = R.string.photo_library_try_later,
        detail = message ?: toString(),
    )
}

/** 整页的说明：一句人话、能做的事、默认收起的详情。 */
@Composable
internal fun PhotoProblemPage(
    problem: Problem,
    onRetry: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    PhotoMessagePage(WontonIcon.Warning, problem.titleText, problem.nextStepText, modifier) {
        onRetry?.let {
            WontonButton(stringResource(R.string.common_retry), it, kind = WontonButtonKind.SECONDARY)
        }
        problem.detail?.let { ProblemDetail(it) }
    }
}

/** 空的库、空的相册、没搜到：图标、一句话、可选的下一步。 */
@Composable
internal fun PhotoEmptyPage(
    title: String,
    hint: String?,
    modifier: Modifier = Modifier,
    actions: @Composable () -> Unit = {},
) {
    PhotoMessagePage(WontonIcon.Image, title, hint, modifier, actions)
}

@Composable
private fun PhotoMessagePage(
    icon: ImageVector,
    title: String,
    hint: String?,
    modifier: Modifier,
    actions: @Composable () -> Unit,
) {
    val colors = WontonTheme.colors
    Column(
        modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // 装饰：下面就是标题。
        Icon(
            icon,
            contentDescription = null,
            Modifier.size(WontonTheme.size.icon),
            tint = colors.textSecondary.toColor(),
        )
        Text(
            title,
            style = WontonTheme.typography.itemTitle.copy(textAlign = TextAlign.Center),
            color = colors.textPrimary.toColor(),
        )
        hint?.let {
            Text(
                it,
                style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.Center),
                color = colors.textSecondary.toColor(),
            )
        }
        actions()
    }
}
