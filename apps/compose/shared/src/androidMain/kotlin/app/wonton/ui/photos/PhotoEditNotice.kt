package app.wonton.ui.photos

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.PhotoEdit
import app.wonton.protocol.PhotoEditKind
import app.wonton.protocol.PhotoEditOutcome
import app.wonton.protocol.PhotosAction
import app.wonton.ui.ProblemDetail
import app.wonton.ui.R
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.delay

private const val NOTICE_MS = 5_000L
private val NOTICE_WIDTH = 480.dp

/** 让开底栏：底栏（或选择中的操作栏）大约这么高。 */
private val ABOVE_BOTTOM_BAR = 96.dp

/**
 * 最近一次写操作的结果，一条浮在底栏上面的轻提示：做成了几项、能撤的带“撤销”；有没成的写清几项没成和头一条的
 * 原因，核心写的原文收进“详情”。全做成的过几秒自己收起，有没成的等人关。
 */
@Composable
internal fun PhotoEditNotice(state: PhotosState, modifier: Modifier = Modifier) {
    val outcome = state.view.lastEdit ?: return
    val dismiss = { state.send(PhotosAction.DismissEditOutcome) }
    val whole = outcome.problem
    val clean = whole == null && outcome.failed.isEmpty()
    if (clean) {
        LaunchedEffect(outcome) {
            delay(NOTICE_MS)
            dismiss()
        }
    }
    val colors = WontonTheme.colors
    val shape = RoundedCornerShape(WontonTheme.radii.card)
    Column(
        modifier.navigationBarsPadding()
            .padding(start = WontonTheme.spacing.s16, end = WontonTheme.spacing.s16, bottom = ABOVE_BOTTOM_BAR)
            .widthIn(max = NOTICE_WIDTH).fillMaxWidth()
            .shadow(WontonTheme.size.menuElevation, shape)
            .background(colors.surface.toColor(), shape)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8)
            .semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(noticeTitle(outcome), style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
                noticeFailure(outcome)?.let {
                    Text(it, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
                }
            }
            if (outcome.kind == PhotoEditKind.TRASH && outcome.done.isNotEmpty() &&
                state.supports(PhotoEditKind.RESTORE)
            ) {
                WontonSmallButton(stringResource(R.string.photo_library_undo), {
                    state.edit(PhotoEdit.Restore(outcome.done))
                })
            }
            if (!clean) WontonSmallButton(stringResource(R.string.common_close), dismiss)
        }
        val detail = whole?.message ?: outcome.failed.joinToString("\n") { "${it.id}: ${it.failure.message}" }
        if (detail.isNotEmpty()) ProblemDetail(detail)
    }
}

/** 第一行：做成了什么；一项都没成就直说没做成。 */
@Composable
private fun noticeTitle(outcome: PhotoEditOutcome): String {
    val done = outcome.done.size
    if (outcome.problem != null || (done == 0 && outcome.failed.isNotEmpty())) {
        return stringResource(R.string.photo_library_edit_failed)
    }
    return when (outcome.kind) {
        PhotoEditKind.FAVORITE -> pluralStringResource(R.plurals.photo_library_done_favorite, done, done)
        PhotoEditKind.UNFAVORITE -> pluralStringResource(R.plurals.photo_library_done_unfavorite, done, done)
        PhotoEditKind.ADD_TO_ALBUM -> pluralStringResource(R.plurals.photo_library_done_added, done, done)
        PhotoEditKind.REMOVE_FROM_ALBUM -> pluralStringResource(R.plurals.photo_library_done_removed, done, done)
        PhotoEditKind.TRASH -> pluralStringResource(R.plurals.photo_library_done_trashed, done, done)
        PhotoEditKind.RESTORE -> pluralStringResource(R.plurals.photo_library_done_restored, done, done)
        PhotoEditKind.CREATE_ALBUM -> stringResource(R.string.photo_library_done_album_created)
        PhotoEditKind.SHARE_ALBUM -> stringResource(R.string.photo_library_done_shared)
    }
}

/** 第二行：几项没成、头一条为什么；整个没成时是那一个原因。 */
@Composable
private fun noticeFailure(outcome: PhotoEditOutcome): String? {
    val whole = outcome.problem
    val first = outcome.failed.firstOrNull()
    return when {
        whole != null -> whole.problem.titleText
        first != null -> {
            val failed = outcome.failed.size
            pluralStringResource(R.plurals.photo_library_failed_items, failed, failed, first.failure.problem.titleText)
        }
        else -> null
    }
}
