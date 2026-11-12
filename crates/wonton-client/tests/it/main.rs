//! 全部集成测试合成一个二进制：每个 `tests/*.rs` 都要单独链接整棵依赖树，九个二进制加起来约 2.2 GB，CI 每次都要写一遍。
//! 新测试加在这里，不新增 `tests/*.rs`。`wonton-core` 没有合成：它的 `image_worker` 要检查本进程的子进程都退出了，不能和别的
//! 测试同住一个进程。

// 同 wonton-matrix：matrix-sdk 的 future 嵌套很深。
#![recursion_limit = "256"]

mod bot_contacts;
mod chat;
mod files;
mod files_cards;
mod files_services;
mod files_support;
mod friends;
mod home;
mod pairing;
mod peers;
mod plugins;
mod port_mapping;
mod servers;
mod session_list;
mod tabs;
mod tabs_bridge;
mod tabs_media;
