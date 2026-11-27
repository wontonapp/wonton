//! 假房间的条目网格：每条的下标、跨日的分界和各自的发送时间。

pub(crate) const ITEMS: usize = 27;
pub(crate) const OWNER: &str = "@owner:home.example";
pub(crate) const ECHO: &str = "@echo:home.example";
/// 最早一条的时间；下标每大一条晚一分钟（下标大 = 屏幕上更靠下 = 更晚）。
pub(super) const FIRST_AT_MS: u64 = 1_792_000_000_000;
/// 从这条起（更靠下的那一段）再往后一天：列表里要有一段跨日，日期分隔那一行才有得看。
pub(super) const DAY_BREAK_INDEX: usize = 11;
pub(super) const DAY_MS: u64 = 86_400_000;
/// 最顶上那条（下标 0）再往前两天，单独占一天：列表最顶上也要有一条日期分隔。
pub(super) const TOP_INDEX: usize = 0;
pub(super) const TOP_DAYS_BACK: u64 = 2 * DAY_MS;

/// 第 `index` 条的发送时间。相邻两条隔一分钟，时间随下标只增不减；第 11 条起是后一天，最顶上的第 0 条再早两天。
pub(crate) fn at_ms(index: usize) -> u64 {
    let later = if index >= DAY_BREAK_INDEX { DAY_MS } else { 0 };
    let earliest = if index == TOP_INDEX { TOP_DAYS_BACK } else { 0 };
    FIRST_AT_MS + index as u64 * 60_000 + later - earliest
}

/// 回复第 5 条的那条（自己发的）；它和之前自己的消息 Echo 都读到了，之后的没有。
pub(crate) const REPLY_INDEX: usize = 21;
/// Echo 带两个选项的那条。
pub(crate) const CHOICES_INDEX: usize = 22;
/// 一条宽表格：比气泡宽，格子上的横向拖动要能滚表格、竖着滑照旧滑时间线。
pub(crate) const TABLE_INDEX: usize = 23;
/// 一条“表情 + 一行字”的短消息，下一条长中文回复引用它：引用块要只有名字加一两行高。
pub(crate) const QUOTED_INDEX: usize = 19;
/// 引用上一条的长中文回复：几段长短不一，有的末行写满，用来查时间戳和正文重叠、长段被截断。
pub(crate) const LONG_REPLY_INDEX: usize = 20;
/// 引用一条不在房间里的消息：跳到它要翻到房间开头，最后给“找不到”。
pub(crate) const MISSING_QUOTE_INDEX: usize = 18;
/// 名字很长的发送者：群聊里名字只许占一行。
pub(crate) const LONG_NAME_INDEX: usize = 22;
/// 删除的消息：菜单里没有任何动作，只剩发送时间；顺便看读屏念不念“已编辑”。
pub(crate) const REDACTED_INDEX: usize = 17;
/// 一条只有 `wonton://` 链接的消息，带查询和结尾斜杠：点它要剥掉后就地打开“其他服务器”。
pub(crate) const APP_LINK_INDEX: usize = 6;
/// 认不得的 `wonton:` 链接（单斜杠写法）：点它要提示一句，不开白页。
pub(crate) const UNKNOWN_APP_LINK_INDEX: usize = 7;
pub(crate) const APP_LINK: &str = "wonton://settings/peers/?from=chat";
pub(crate) const UNKNOWN_APP_LINK: &str = "wonton:tab/from-the-future";
/// 自己发的、发不出去的一条：旁边红色感叹号的点击范围要看它。
pub(crate) const FAILED_SEND_INDEX: usize = 24;
/// Echo 带一个发送失败的选项：失败原因和重试要看它。
pub(crate) const FAILED_CHOICE_INDEX: usize = 25;
/// 失败选项的原因，界面上应当收在“详情”里。
pub(crate) const FAILED_CHOICE_REASON: &str = "Server timed out while sending";
/// 这条下面挂着一张旁聊卡，点进去是旁聊页：界面测试用它查顶栏下面的叠放顺序（ADR-0093）。
/// 就挂在进房间的落点那条上，不用翻页；照片条不走 `decorate`，别选照片的下标。
pub(crate) const SIDE_CHAT_TOP_INDEX: usize = 2;
// 中文假数据放在数据文件里：它是界面测试的输入，不是界面文字，界面文字检查（ADR-0043）只管源码里的字符串。
pub(super) const QUOTED_TEXT: &str = include_str!("../ui_fake_text/quoted.txt");
pub(super) const LONG_REPLY: &str = include_str!("../ui_fake_text/long_reply.txt");
/// 带旁聊卡片的那条：点了进那个旁聊。
pub(crate) const SIDE_ROOT_INDEX: usize = 5;
/// 旁聊里的条数。
pub(super) const SIDE_ITEMS: usize = 4;
/// 每隔几条来一条比屏幕高的；最后一条也是。
pub(super) const TALL_EVERY: usize = 4;
pub(super) const TALL_PARAGRAPHS: usize = 60;
/// 最后一条消息的最后一段：UI 测试按它找真正的底。
pub(super) const LAST_LINE: &str = "Fixture last line";
/// 第 22 条的发送者名字，长到一行放不下。
pub(super) const LONG_NAME: &str = "Echo the group bot with a really long account name";
/// 最后一条是自己刚发、还没送达的：待发脚注“已保存，等待送达”和时钟由它验。
pub(super) const SENDING_INDEX: usize = ITEMS - 1;
