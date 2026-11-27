package app.wonton.ui.video

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import app.wonton.client.VideoSort
import app.wonton.client.VideoWallQuery
import app.wonton.ui.ProblemRow
import app.wonton.ui.R
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 海报墙的排序和筛选：排序、只看没看过的、类型。类型没取到时说原因，点了再取。改了马上生效。 */
@Composable
fun VideoFilterSheet(
    model: VideoModel,
    query: VideoWallQuery,
    onChange: (VideoWallQuery) -> Unit,
    onDismiss: () -> Unit,
) {
    var genres by remember { mutableStateOf<VideoLoad<List<String>>>(VideoLoad.Loading) }
    var attempt by remember { mutableIntStateOf(0) }
    LaunchedEffect(attempt) {
        genres = model.genres(query.library).fold({ VideoLoad.Loaded(it) }, { VideoLoad.Failed(it.videoProblem) })
    }
    WontonSheet(onDismiss, stringResource(R.string.video_filter)) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
                .padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            Heading(stringResource(R.string.video_sort))
            listOf(
                VideoSort.TITLE to R.string.video_sort_title,
                VideoSort.ADDED to R.string.video_sort_added,
                VideoSort.RELEASED to R.string.video_sort_released,
                VideoSort.RATING to R.string.video_sort_rating,
            ).forEach { (sort, label) ->
                WontonRadioRow(stringResource(label), query.sort == sort, true, { onChange(query.copy(sort = sort)) })
            }
            Row(
                Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    stringResource(R.string.video_unplayed_only),
                    Modifier.weight(1f),
                    style = WontonTheme.typography.body,
                    color = WontonTheme.colors.textPrimary.toColor(),
                )
                WontonSwitch(query.unplayedOnly, { onChange(query.copy(unplayedOnly = it)) })
            }
            Genres(genres, query, onChange) { attempt += 1 }
        }
    }
}

@Composable
private fun Genres(
    genres: VideoLoad<List<String>>,
    query: VideoWallQuery,
    onChange: (VideoWallQuery) -> Unit,
    retry: () -> Unit,
) {
    when (genres) {
        VideoLoad.Loading -> Unit
        is VideoLoad.Failed -> {
            Heading(stringResource(R.string.video_genre))
            ProblemRow(genres.problem, retry)
        }
        is VideoLoad.Loaded -> if (genres.value.isNotEmpty()) {
            Heading(stringResource(R.string.video_genre))
            WontonRadioRow(
                stringResource(R.string.video_genre_all), query.genre == null, true,
                { onChange(query.copy(genre = null)) },
            )
            genres.value.forEach { genre ->
                WontonRadioRow(genre, query.genre == genre, true, { onChange(query.copy(genre = genre)) })
            }
        }
    }
}

@Composable
private fun Heading(text: String) {
    Text(
        text,
        Modifier.semantics { heading() },
        style = WontonTheme.typography.supporting,
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}
