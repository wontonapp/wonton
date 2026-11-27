//! 条目主人、相册成员显示成什么名字：共享名单里有这个人就用他的显示名，没有就用 Matrix ID 的本地部分。
//! 名单和条目分头取、谁先回不定，名单也可能取不成被清掉，所以每次改完视图都整体重标一遍。

use std::collections::HashMap;

use wonton_protocol::media::library_view::{
    PhotoAlbumOwner, PhotoAlbumRow, PhotoTile, PhotoUser, PhotosViewModel,
};

pub(crate) fn name_people(view: &mut PhotosViewModel) {
    let PhotosViewModel {
        share_targets,
        timeline,
        albums,
        album,
        search,
        detail,
        ..
    } = view;
    let names: HashMap<&str, &str> = share_targets
        .people
        .iter()
        .filter_map(|person| {
            let name = person.display_name.as_deref()?.trim();
            (!name.is_empty()).then_some((person.user_id.as_str(), name))
        })
        .collect();
    let tiles = timeline
        .months
        .iter_mut()
        .flat_map(|month| &mut month.days)
        .flat_map(|day| &mut day.tiles)
        .chain(album.iter_mut().flat_map(|album| &mut album.page.tiles))
        .chain(search.iter_mut().flat_map(|search| &mut search.page.tiles))
        .chain(detail.iter_mut().filter_map(|detail| detail.tile.as_mut()));
    for tile in tiles {
        name_tile(tile, &names);
    }
    let rows = albums
        .mine
        .iter_mut()
        .chain(&mut albums.household)
        .chain(&mut albums.shared_with_me)
        .chain(album.iter_mut().filter_map(|album| album.album.as_mut()));
    for row in rows {
        name_row(row, &names);
    }
}

fn name_tile(tile: &mut PhotoTile, names: &HashMap<&str, &str>) {
    if let Some(owner) = &mut tile.owner {
        name(owner, names);
    }
}

fn name_row(row: &mut PhotoAlbumRow, names: &HashMap<&str, &str>) {
    if let PhotoAlbumOwner::Person { user } = &mut row.owner {
        name(user, names);
    }
    for member in &mut row.members {
        name(member, names);
    }
}

fn name(user: &mut PhotoUser, names: &HashMap<&str, &str>) {
    match names.get(user.user_id.as_str()) {
        Some(known) => {
            if user.name != *known {
                (*known).clone_into(&mut user.name);
            }
        }
        None => {
            let fallback = PhotoUser::unnamed(&user.user_id).name;
            if user.name != fallback {
                user.name = fallback;
            }
        }
    }
}
