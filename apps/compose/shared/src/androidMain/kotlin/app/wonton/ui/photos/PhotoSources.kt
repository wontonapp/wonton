package app.wonton.ui.photos

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.PhotoSource
import app.wonton.protocol.PhotoSourceState
import app.wonton.protocol.PhotosAction
import app.wonton.ui.LoadingView
import app.wonton.ui.Problem
import app.wonton.ui.R
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.WontonIcon

/**
 * 用哪个相册应用还没定下来时的整页（`PhotosViewModel.sources`）：在问核心时转圈；一个能用的都没有时直说没装，
 * 这不是出错；有几家、没定默认时列出来让人挑一个，挑的由核心记成这台服务器上的默认；没问成时说原因，带重试和详情。
 */
@Composable
internal fun PhotoSourcesPage(state: PhotosState, onClose: () -> Unit) {
    val sources = state.view.sources
    Column(Modifier.fillMaxSize()) {
        WontonPushedBar(stringResource(R.string.photo_library_tab_photos), onClose)
        when (sources.state) {
            PhotoSourceState.NOT_INSTALLED -> PhotoEmptyPage(
                stringResource(R.string.photo_library_not_installed_title),
                stringResource(R.string.photo_library_not_installed_hint),
            )
            PhotoSourceState.CHOOSE -> SourceChoices(sources.candidates) {
                state.send(PhotosAction.ChooseSource(it.id))
            }
            PhotoSourceState.FAILED -> PhotoProblemPage(
                sources.problem?.problem ?: Problem(
                    titleRes = R.string.photo_library_failed_title,
                    nextStepRes = R.string.photo_library_try_later,
                ),
                onRetry = { state.open() },
            )
            PhotoSourceState.CHECKING, PhotoSourceState.READY -> LoadingView()
        }
    }
}

@Composable
private fun SourceChoices(candidates: List<PhotoSource>, onChoose: (PhotoSource) -> Unit) {
    val colors = WontonTheme.colors
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).navigationBarsPadding()) {
        Text(
            stringResource(R.string.photo_library_choose_source),
            Modifier.padding(WontonTheme.spacing.pageInset),
            style = WontonTheme.typography.sectionTitle,
            color = colors.textPrimary.toColor(),
        )
        candidates.forEach { source ->
            SheetRow(source.title, null, { onChoose(source) }) {
                Box(
                    Modifier.size(WontonTheme.size.taskIcon)
                        .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control)),
                    contentAlignment = Alignment.Center,
                ) {
                    // 装饰：应用名就在右边。
                    Icon(
                        WontonIcon.Image,
                        contentDescription = null,
                        Modifier.size(WontonTheme.size.icon),
                        tint = colors.textPrimary.toColor(),
                    )
                }
            }
        }
    }
}
