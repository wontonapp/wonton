package app.wonton.ui.photos

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.MediaKind
import app.wonton.protocol.PhotoPerson
import app.wonton.protocol.PhotoSearch
import app.wonton.protocol.PhotosAction
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonChip
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Person
import app.wonton.ui.icons.WontonIcon

private val FACE = 64.dp
private val FACE_CELL = 76.dp
private val FACE_RING = 3.dp

/** 搜索页的筛选：人物头像一横条，下面一排胶囊（时间、地点、照片、视频、收藏）。改了哪项就把新的条件交回去。 */
@Composable
internal fun PhotoSearchFilters(
    state: PhotosState,
    query: PhotoSearch,
    time: TimeFilter,
    onChange: (PhotoSearch, TimeFilter) -> Unit,
) {
    var pickingTime by remember { mutableStateOf(false) }
    var pickingPlace by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        PeopleRow(state, query.people) { onChange(query.copy(people = it), time) }
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())
                .padding(horizontal = WontonTheme.spacing.pageInset),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            WontonChip(timeLabel(time, query), time != TimeFilter.ANY) { pickingTime = true }
            WontonChip(
                query.place ?: stringResource(R.string.photo_library_filter_place),
                query.place != null,
            ) { pickingPlace = true }
            KindChip(R.string.photo_library_filter_photos, MediaKind.Photo, query) { onChange(it, time) }
            KindChip(R.string.photo_library_filter_videos, MediaKind.Video, query) { onChange(it, time) }
            WontonChip(stringResource(R.string.photo_library_filter_favorites), query.favorite == true) {
                onChange(query.copy(favorite = if (query.favorite == true) null else true), time)
            }
        }
    }
    if (pickingTime) {
        PhotoTimeSheet(time, onDismiss = { pickingTime = false }) { picked, from, to ->
            onChange(query.copy(from = from, to = to), picked)
        }
    }
    if (pickingPlace) {
        PlaceDialog(query.place.orEmpty(), { pickingPlace = false }) { place ->
            pickingPlace = false
            onChange(query.copy(place = place.trim().ifEmpty { null }), time)
        }
    }
}

@Composable
private fun KindChip(title: Int, kind: MediaKind, query: PhotoSearch, onChange: (PhotoSearch) -> Unit) {
    val on = query.kind == kind
    WontonChip(stringResource(title), on) { onChange(query.copy(kind = if (on) null else kind)) }
}

/** 相册应用认出的有名字的人：点头像按这个人筛，再点取消，可以选几个。名单取不到时留一条轻提示。 */
@Composable
private fun PeopleRow(state: PhotosState, picked: List<String>, onPick: (List<String>) -> Unit) {
    val people = state.view.people
    val failure = people.problem
    if (people.people.isEmpty()) {
        if (failure != null) {
            Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) {
                ProblemRow(failure.problem) { state.send(PhotosAction.Refresh) }
            }
        }
        return
    }
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        people.people.forEach { person ->
            val on = person.id in picked
            PersonFace(state, person, on) { onPick(if (on) picked - person.id else picked + person.id) }
        }
    }
}

@Composable
private fun PersonFace(state: PhotosState, person: PhotoPerson, on: Boolean, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Column(
        Modifier.width(FACE_CELL).clickable(role = Role.Checkbox, onClick = onClick).semantics { selected = on },
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        val ring = if (on) Modifier.border(FACE_RING, colors.primary.toColor(), CircleShape) else Modifier
        Box(
            Modifier.size(FACE).then(ring).padding(if (on) FACE_RING * 2 else 0.dp).clip(CircleShape)
                .background(colors.surfaceSubtle.toColor())
                .then(Modifier.border(WontonTheme.size.hairline, colors.separator.toColor(), CircleShape)),
        ) {
            // 装饰：名字就在底下。头像还没到或取不到时露出底下这个图标。
            Icon(
                WontonIcon.Person,
                contentDescription = null,
                Modifier.align(Alignment.Center).size(WontonTheme.size.icon),
                tint = colors.textSecondary.toColor(),
            )
            PhotoImage(state.personRef(person.id), null, null, Modifier.matchParentSize())
        }
        Text(
            person.name,
            style = WontonTheme.typography.caption.copy(textAlign = TextAlign.Center),
            color = (if (on) colors.textPrimary else colors.textSecondary).toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** 按地点筛：城市、地区或国家，写什么搜什么。 */
@Composable
private fun PlaceDialog(current: String, onDismiss: () -> Unit, onDone: (String) -> Unit) {
    var place by remember { mutableStateOf(current) }
    val hint = stringResource(R.string.photo_library_filter_place_hint)
    WontonDialog(
        stringResource(R.string.photo_library_filter_place),
        onDismiss = onDismiss,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(stringResource(R.string.common_done), WontonButtonKind.PRIMARY) { onDone(place) },
        ),
        detail = { WontonTextField(place, { place = it }, Modifier.fillMaxWidth(), placeholder = hint, label = hint) },
    )
}
