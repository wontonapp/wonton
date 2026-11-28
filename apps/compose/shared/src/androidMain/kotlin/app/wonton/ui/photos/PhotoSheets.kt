package app.wonton.ui.photos

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.wonton.protocol.PhotoAlbumRow
import app.wonton.protocol.PhotoEdit
import app.wonton.protocol.PhotoEditKind
import app.wonton.protocol.PhotoRendition
import app.wonton.ui.R
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.WontonIcon

/** 选好的几项加进哪个相册：自己的和家庭的相册，顶上是新建一个。 */
@Composable
internal fun PhotoAlbumPicker(state: PhotosState, ids: List<String>, onDismiss: () -> Unit) {
    var naming by remember { mutableStateOf(false) }
    if (naming) {
        NewAlbumDialog(onDismiss) { title ->
            state.edit(PhotoEdit.CreateAlbum(title, ids, emptyList()))
            onDismiss()
        }
        return
    }
    val albums = state.view.albums
    WontonSheet(onDismiss, stringResource(R.string.photo_library_add_to_album)) { close ->
        SheetList {
            if (state.supports(PhotoEditKind.CREATE_ALBUM)) {
                SheetRow(stringResource(R.string.photo_library_new_album), null, { naming = true }) {
                    val shape = RoundedCornerShape(WontonTheme.radii.control)
                    Box(
                        Modifier.size(WontonTheme.size.fileThumbnail)
                            .background(WontonTheme.colors.surfaceSubtle.toColor(), shape),
                        contentAlignment = Alignment.Center,
                    ) {
                        // 装饰：行的文字在右边。
                        Icon(
                            WontonIcon.Add,
                            contentDescription = null,
                            Modifier.size(WontonTheme.size.icon),
                            tint = WontonTheme.colors.textPrimary.toColor(),
                        )
                    }
                }
            }
            if (state.supports(PhotoEditKind.ADD_TO_ALBUM)) {
                (albums.mine + albums.household).forEach { album ->
                    val count = album.count.toInt()
                    SheetRow(
                        album.title,
                        pluralStringResource(R.plurals.photo_library_items, count, count),
                        { close { state.edit(PhotoEdit.AddToAlbum(album.id, ids)) } },
                    ) { AlbumCover(state, album, Modifier.size(WontonTheme.size.fileThumbnail)) }
                }
            }
        }
    }
}

/** 相册的封面；没有封面的画底色。 */
@Composable
internal fun AlbumCover(state: PhotosState, album: PhotoAlbumRow, modifier: Modifier = Modifier) {
    val shape = RoundedCornerShape(WontonTheme.radii.control)
    val cover = album.cover
    Box(modifier.clip(shape).background(WontonTheme.colors.surfaceSubtle.toColor()), Alignment.Center) {
        // 装饰：相册名就在旁边。没有封面、封面还没到或取不到时露出底下这个图标。
        Icon(
            WontonIcon.Image,
            contentDescription = null,
            Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
        if (cover != null) {
            PhotoImage(state.ref(cover, PhotoRendition.THUMBNAIL), null, null, Modifier.matchParentSize())
        }
    }
}

/** 面板里一长列能滚的行。 */
@Composable
internal fun ColumnScope.SheetList(content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()), content = content)
}

@Composable
internal fun SheetRow(
    title: String,
    subtitle: String?,
    onClick: () -> Unit,
    role: Role = Role.Button,
    trailing: (@Composable () -> Unit)? = null,
    leading: @Composable () -> Unit,
) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target + WontonTheme.spacing.s12)
            .clickable(role = role, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s4),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        leading()
        Column(Modifier.weight(1f)) {
            Text(title, style = WontonTheme.typography.body, color = colors.textPrimary.toColor(), maxLines = 1)
            subtitle?.let {
                Text(
                    it,
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                    maxLines = 1,
                )
            }
        }
        trailing?.invoke()
    }
}

/** 给新相册起名。 */
@Composable
internal fun NewAlbumDialog(onDismiss: () -> Unit, onCreate: (String) -> Unit) {
    var title by remember { mutableStateOf("") }
    val name = stringResource(R.string.photo_library_album_name)
    WontonDialog(
        stringResource(R.string.photo_library_new_album),
        onDismiss = onDismiss,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(stringResource(R.string.photo_library_create), WontonButtonKind.PRIMARY) {
                if (title.isNotBlank()) onCreate(title.trim())
            },
        ),
        detail = { WontonTextField(title, { title = it }, Modifier.fillMaxWidth(), placeholder = name, label = name) },
    )
}
