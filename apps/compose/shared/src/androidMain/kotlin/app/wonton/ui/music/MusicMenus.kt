package app.wonton.ui.music

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.wonton.protocol.AlbumSort
import app.wonton.protocol.MusicPage
import app.wonton.ui.R
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.More
import app.wonton.ui.icons.Search
import app.wonton.ui.icons.WontonIcon

/** 向右的箭头转成向下：下拉菜单的提示。 */
private const val DOWN_DEGREES = 90f

/** 入口页右上角：搜索，和网页胶囊同样的菜单（网页版、加到我的应用、详情）。 */
@Composable
internal fun MusicRootMenu(root: MusicRootActions) {
    val music = LocalMusic.current
    var open by remember { mutableStateOf(false) }
    WontonIconButton(WontonIcon.Search, stringResource(R.string.music_search), { music.open(MusicPage.Search) })
    Box {
        WontonIconButton(WontonIcon.More, stringResource(R.string.tab_page_menu), { open = true })
        DropdownMenu(open, { open = false }, containerColor = WontonTheme.colors.surface.toColor()) {
            MenuItem(R.string.music_web_version) { open = false; root.openWeb() }
            MenuItem(R.string.myapp_add) { open = false; root.addToMyApps() }
            MenuItem(R.string.tab_menu_details) { open = false; root.details() }
        }
    }
}

/** 换专辑排序：原地换掉这一页，不多一层返回。 */
@Composable
internal fun MusicSortMenu(sort: AlbumSort) {
    val music = LocalMusic.current
    var open by remember { mutableStateOf(false) }
    Box {
        SortButton(sort) { open = true }
        DropdownMenu(open, { open = false }, containerColor = WontonTheme.colors.surface.toColor()) {
            AlbumSort.entries.forEach { option ->
                DropdownMenuItem(
                    text = { Text(stringResource(sortName(option))) },
                    onClick = {
                        open = false
                        if (option != sort) music.replace(MusicPage.Albums(option))
                    },
                    trailingIcon = if (option == sort) {
                        {
                            Icon(
                                WontonIcon.Check,
                                // 装饰：勾表示当前的排序，名字就在左边。
                                contentDescription = null,
                                Modifier.size(WontonTheme.size.iconSmall),
                            )
                        }
                    } else null,
                )
            }
        }
    }
}

/** 当前的排序写在按钮上，点开换。 */
@Composable
private fun SortButton(sort: AlbumSort, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.clip(RoundedCornerShape(WontonTheme.radii.control))
            .clickable(onClickLabel = stringResource(R.string.music_sort), role = Role.Button, onClick = onClick)
            .heightIn(min = WontonTheme.size.target).padding(horizontal = WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        Text(
            stringResource(sortName(sort)),
            style = WontonTheme.typography.supporting,
            color = colors.textPrimary.toColor(),
        )
        Icon(
            WontonIcon.Forward,
            // 装饰：表示能展开。
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconTiny).rotate(DOWN_DEGREES),
            tint = colors.textPrimary.toColor(),
        )
    }
}

@Composable
private fun MenuItem(label: Int, onClick: () -> Unit) {
    DropdownMenuItem(text = { Text(stringResource(label)) }, onClick = onClick)
}

private fun sortName(sort: AlbumSort): Int = when (sort) {
    AlbumSort.NEWEST -> R.string.music_sort_newest
    AlbumSort.RECENT -> R.string.music_sort_recent
    AlbumSort.FREQUENT -> R.string.music_sort_frequent
    AlbumSort.NAME -> R.string.music_sort_name
    AlbumSort.ARTIST -> R.string.music_sort_artist
    AlbumSort.RANDOM -> R.string.music_sort_random
}
