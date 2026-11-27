//! 假数据入口（`ui_fake.rs`）里的房间和本机网页：一条够长的时间线，一张带全屏固定浮层的面板网页，
//! 和一张右上角有固定按钮、整页打开的网页。
//!
//! 时间线里有几条比屏幕还高的消息，落点在靠前的一条上（进房间停在别处、点“回到底部”再上推，是开发栈上
//! 房间走屏的真实顺序）。面板网页由回环地址上的一个小服务给：公网网页那一类（`TabRef::Url`）不经隧道，
//! 壳直接连它；页面本身能滚，上面盖着一层全屏的固定浮层，右上角一个关闭按钮，照 Navidrome 的手机播放器摆。
//! 整页的那张（[`corner`]）照 Immich、Actual 这类现成应用摆：顶上、底下各一条固定的栏，右上角一个按钮，
//! 出口胶囊压不压得到它由界面测试看（ADR-0095“铺满”）；它声明了 `theme-color`，页面上的按钮能换掉或去掉这个声明，
//! 壳让出来的顶条跟不跟页面的颜色也由界面测试看。
//!
//! 夹具按职责分在 `ui_fake_room/` 下：条目网格（`layout.rs`）、整批时间线（`timeline.rs`）、
//! 逐条内容（`item.rs`）、通用消息构造（`message.rs`）、照片字节（`media.rs`）、本机网页（`pages.rs`）。

mod item;
mod layout;
pub(super) mod media;
mod message;
mod pages;
pub(super) mod timeline;

#[allow(unused_imports)]
// 假房间内部的索引常量拆分后没有别的调用方，但旧路径 `ui_fake_room::X` 要保留。
pub(super) use layout::{
    APP_LINK, APP_LINK_INDEX, CHOICES_INDEX, ECHO, FAILED_CHOICE_INDEX, FAILED_CHOICE_REASON,
    FAILED_SEND_INDEX, ITEMS, LONG_NAME_INDEX, LONG_REPLY_INDEX, MISSING_QUOTE_INDEX, OWNER,
    QUOTED_INDEX, REDACTED_INDEX, REPLY_INDEX, SIDE_CHAT_TOP_INDEX, SIDE_ROOT_INDEX, TABLE_INDEX,
    UNKNOWN_APP_LINK, UNKNOWN_APP_LINK_INDEX, at_ms,
};
pub(super) use media::media;
pub(super) use message::text_message;
pub(super) use pages::corner;
#[allow(unused_imports)] // `side_card` 只有测试用。
pub(super) use timeline::{side_card, side_timeline, started_timeline, timeline};
