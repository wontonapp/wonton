//! 照片分组：照片同组带位置、字节加载得到。

use super::super::media::media;
use super::super::timeline::timeline;
use super::*;
use wonton_protocol::TimelineDiff;

#[test]
fn photos_come_as_one_group_with_loadable_bytes() {
    let update = timeline("!ui-fake:home.example");
    let [TimelineDiff::Reset { items }] = update.diffs.as_slice() else {
        panic!("the first batch must be a reset");
    };
    let images: Vec<&TimelineMessage> = items
        .iter()
        .filter_map(|item| match &item.content {
            TimelineContent::Message { message }
                if matches!(message.body, MessageContent::Image { .. }) =>
            {
                Some(message)
            }
            _ => None,
        })
        .collect();
    // 组里三张（同组、按 index 排好）加一张没组的单张。
    let grouped: Vec<&TimelineMessage> = images
        .iter()
        .copied()
        .filter(|m| m.group.is_some())
        .collect();
    assert_eq!(grouped.len(), PHOTO_GROUP_COUNT);
    for (position, message) in grouped.iter().enumerate() {
        let group = message
            .group
            .as_ref()
            .expect("a grouped photo carries its group");
        assert_eq!(
            (group.index, group.count),
            (position as u32, PHOTO_GROUP_COUNT as u32)
        );
        let MessageContent::Image { image } = &message.body else {
            panic!("a photo message must carry an image");
        };
        assert!(
            media(&image.media.token).is_some(),
            "{} must load",
            image.media.token
        );
        assert_eq!(image.thumbnail, Some(image.media.clone()));
    }
    let solo = images
        .iter()
        .find(|m| m.group.is_none())
        .expect("one photo is ungrouped");
    let MessageContent::Image { image } = &solo.body else {
        panic!("a photo message must carry an image");
    };
    assert!(
        media(&image.media.token).is_some(),
        "{} must load",
        image.media.token
    );
}
