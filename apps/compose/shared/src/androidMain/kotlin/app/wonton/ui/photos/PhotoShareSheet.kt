package app.wonton.ui.photos

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.wonton.protocol.PhotoAlbumRow
import app.wonton.protocol.MediaFailureCode
import app.wonton.protocol.PhotoEdit
import app.wonton.protocol.PhotoShareTarget
import app.wonton.protocol.PhotoShareTargets
import app.wonton.ui.Problem
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.emphasisFill
import app.wonton.ui.design.toColor

/**
 * 共享给家人：勾家里的成员。选好的几项先起个相册名、建成共享相册；已有的相册直接改共享的人（已经共享给的先勾上）。
 * 能选谁由核心给（`PhotosViewModel.share_targets`）；取不到时照它的原因说，不给空名单装作没人。
 */
@Composable
internal fun PhotoShareSheet(state: PhotosState, target: ShareTarget, onDismiss: () -> Unit) {
    val targets = state.view.shareTargets
    val shared = (target as? ShareTarget.Album)?.album?.members.orEmpty().map { it.userId }
    var picked by remember { mutableStateOf(shared.toSet()) }
    var title by remember { mutableStateOf("") }
    WontonSheet(onDismiss, stringResource(R.string.photo_library_share_title)) { close ->
        if (target is ShareTarget.NewAlbum && targets.people.isNotEmpty()) {
            val name = stringResource(R.string.photo_library_album_name)
            WontonTextField(
                title, { title = it },
                Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.pageInset),
                placeholder = name, label = name,
            )
        }
        SheetList {
            targets.people.forEach { person ->
                val on = person.userId in picked
                SheetRow(
                    person.label,
                    null,
                    { picked = if (on) picked - person.userId else picked + person.userId },
                    role = Role.Checkbox,
                    trailing = { SelectionMark(on) },
                ) { MemberInitial(person) }
            }
        }
        if (targets.people.isEmpty()) ShareTargetsStatus(targets)
        val ready = picked.isNotEmpty() && (target is ShareTarget.Album || title.isNotBlank())
        if (targets.people.isNotEmpty()) WontonButton(
            stringResource(R.string.photo_library_share),
            {
                close {
                    state.edit(
                        when (target) {
                            is ShareTarget.NewAlbum -> PhotoEdit.CreateAlbum(title.trim(), target.ids, picked.toList())
                            is ShareTarget.Album -> PhotoEdit.ShareAlbum(target.album.id, picked.toList())
                        },
                    )
                }
            },
            Modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset),
            enabled = ready,
        )
    }
}

/** 共享什么：选好的几项（建一个新的共享相册），或一个已有的相册。 */
internal sealed interface ShareTarget {
    data class NewAlbum(val ids: List<String>) : ShareTarget

    data class Album(val album: PhotoAlbumRow) : ShareTarget
}

/** 名单是空的时候：在取、没取成（照核心给的原因说，原文收在“详情”）、家里没有别人。 */
@Composable
private fun ShareTargetsStatus(targets: PhotoShareTargets) {
    // 核心回没权限说的是这个人不能共享（比如还没加入这台服务器），不是相册应用没开给他：用这里自己的一句，
    // 核心的原文照样收在“详情”。
    val failure = targets.problem?.let { raw ->
        if (raw.code == MediaFailureCode.PermissionDenied) {
            Problem(titleRes = R.string.photo_library_share_denied, detail = raw.message)
        } else raw.problem
    }
    val inset = Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8)
    when {
        failure != null -> Box(inset) { ProblemRow(failure) }
        targets.loading -> Box(inset.fillMaxWidth(), contentAlignment = Alignment.Center) {
            CircularProgressIndicator(color = WontonTheme.colors.textSecondary.toColor())
        }
        else -> Text(
            stringResource(R.string.photo_library_family_empty),
            inset,
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

/** 显示的名字：没填显示名时用 Matrix ID 里 `@` 和 `:` 之间那一段。 */
private val PhotoShareTarget.label: String
    get() = displayName?.takeIf { it.isNotBlank() } ?: userId.matrixLocalPart()

@Composable
private fun MemberInitial(person: PhotoShareTarget) {
    Box(Modifier.size(WontonTheme.size.taskIcon).emphasisFill(CircleShape), contentAlignment = Alignment.Center) {
        Text(
            person.label.take(1).uppercase(),
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.onPrimary.toColor(),
        )
    }
}
