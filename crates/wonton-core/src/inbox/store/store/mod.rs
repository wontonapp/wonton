//! 收件箱的存储、生命周期、上限和答案。对外只有 [`Inbox`] 一个入口。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use super::*;
use crate::inbox::MAX_WAITERS_PER_ITEM;
use crate::inbox::page::{self, PageTokens};
use tokio::sync::oneshot;
use wonton_protocol::{InboxAnswer, InboxAnswerInput, InboxItem, InboxPayload, InboxStatus};

mod cleanup;
mod items;
mod lifecycle;
mod limits;
mod open;
