# vendor

带补丁的第三方 crate。根 `Cargo.toml` 的 `[patch.crates-io]` 指向这里，`[workspace]` 把它们排除在外：不跑 fmt、clippy
和我们的测试，行数门禁也跳过 `vendor/`（上游的文件不归我们的拆分规则管）。

## matrix-sdk 0.19.1（Apache-2.0，`matrix-sdk/LICENSE`）

crates.io 上的包原样导入：sha256 `2afc652a144822f6fbe24084e08bc59cb483fd0b504396d492131dec0749396e`，上游提交见
`matrix-sdk/.cargo_vcs_info.json`。包里没有 LICENSE，从上游同一提交取来。

### SAS 缺秘密时补要

`src/encryption/mod.rs` 加了公开方法 `Encryption::query_missing_secrets_from_other_sessions`，转调
matrix-sdk-crypto 已经公开的 `OlmMachine::query_missing_secrets_from_other_sessions`，向自己的其他设备再要一次缺的秘密，
再调 SDK 内部的 `send_outgoing_requests` 当场发出。只排队不发的话，请求要等下一次同步才发，可能是 30 秒的长轮询，
取回密钥的期限就过了。

为什么：自我验证时，新设备收到对方的 MAC 和 `done` 就同时发自己的 `done` 和 `m.secret.request`；要秘密的请求单独一轮同步先到旧设备时，
旧设备还没把新设备记成已验证，回 “device isn't trusted” 直接丢掉，不排队也不补发，新设备就一直缺那个秘密。
实测和时序见 [ADR-0046](../docs/adr/0046-device-verification.md)“取回密钥”一节。matrix-sdk 只在 `testing` feature 下
暴露 `OlmMachine`，正式代码没法补要；`base_client()` 绕过存储配置和跨进程锁，不能用。
这条补丁只服务 SAS 验证后的秘密请求；输入恢复密钥走秘密存储，不调用它。

### 恢复完成不被旧状态检查覆盖

`src/encryption/recovery/mod.rs` 的 `all_known_secrets_available` 在查询服务器的备份禁用标志后，重新读取本地备份是否启用。
实测：设备列表刷新先看到本地备份未启用，等待 `m.key_backup` 的 HTTP 应答期间，另一条恢复密钥操作已导入全部秘密、启用备份；
旧检查随后回写 `Incomplete`，界面又要求输入恢复密钥。网络后读本地状态保留原来的“已启用或明确禁用”判断，不重试、不改变等待上限。
回归用真实 Tuwunel、只延迟真实 HTTP 应答，见 `crates/wonton-matrix/tests/tuwunel/recovery_order.rs`。

### 同步起步的日志

`src/sliding_sync/mod.rs` 的 `generate_sync_request` 加几条 `debug!`，记下从拿到 `pos` 到发出请求之间每一步
（读回存下的位置、取 Olm 机器、读房间订阅）。这一段原本没有任何日志，同步卡在起步时只能看见前后两个时间点，分不出卡在哪，
补上才能用 `WONTON_TEST_LOG` 定位。

### 观察 Client 生命周期的弱引用

`src/client/mod.rs` 加了公开方法 `Client::downgrade`，并把 `WeakClient` 从 `pub(crate)` 放宽为 `pub`、
在 `src/lib.rs` 转出 crate 根（`client` 模块本身仍是私有的，crate 根转出是唯一可达路径）。
回归测试要从外面观察客户端是否被 SDK 内部钉住：弱引用 `get()` 在全部强引用
丢掉后应取回 `None`，见 `crates/wonton-matrix/tests/tuwunel/client_lifetime.rs`。

### LatestEvents 后台任务不持有 Client

`src/latest_events/mod.rs`：`LatestEvents::new` 原来把 `EventCache` 和 `SendQueue` 整个 move 进
`listen_to_updates_task`（行 115-121 的 spawn 调用）；现在 spawn 前先调
`event_cache.subscribe_to_room_generic_updates()` 和 `send_queue.subscribe()`，只把两个
`broadcast::Receiver` 传进任务。`listen_to_updates_task`（行 464 起）签名相应从收两个值改为收两个
receiver，任务体开头两行订阅删掉，`select!` 循环不动；`room_info_updates` 本来就以 receiver 传入，这次对齐同款。

为什么：`SendQueue { client: Client }` 持强引用，而任务唯一的退出条件是三条 broadcast 通道关闭，
三条通道的 sender 全在 `ClientInner` 里（EventCache 的 `generic_update_sender`、
`send_queue_data.global_update_sender`、room info broadcaster）——任务把客户端按住就永远等不到
通道关闭，同步过一次的 Client 从此释放不掉（#816），进程内重建会话时旧实例全部滞留。改成 receiver 后，
客户端 drop 时 `_listen_task_handle` 的 `AbortOnDrop` 生效、通道随之关闭，两条退出路径都通。
回归见 `crates/wonton-matrix/tests/tuwunel/client_lifetime.rs`（修前红、修后绿）。

### 上传进度节流、发送更新通道留足缓冲

`src/send_queue/progress.rs` 的进度转发从“HTTP 客户端每 8 KB 一条”改成每 100 ms 最多一条，传完的那条不等间隔；
`src/send_queue/mod.rs` 里每个房间的发送更新广播从 32 格放到 256 格。

为什么：HTTP 客户端每 8 KB 报一次进度，8 MB 的附件是 1024 次。环回上不节流时一次上传就有十几条背靠背的更新
（`eyeball` 的订阅按调度合并，块越小合并越多），每一条都要时间线改一次本地回显、给订阅方发一次 `Set`。进度又和其他
发送更新（本地回显、发出、失败）共用同一条广播，满了挤掉最旧的；接收方慢一阵子、同时又有别的消息在发时，被挤掉的
可能是最后的 100% 和“已发送”，那条消息就一直停在“发送中”，直到服务器回显经同步补上。上游 0.19.1 和 `main`
都还是每块一条、容量 32。容量按节流后的速率给接收方约 25 秒余地，一串几十条别的更新也挤不掉写完的进度。

行为回归见 `crates/wonton-matrix/tests/tuwunel/send_queue.rs`：`image_progress_updates_are_throttled` 发 8 MB 的图，
断言进度条数不超过“每 100 ms 一条再加几条不等间隔的（缩略图、原图各自的开头和传完）”；`image_final_state_waits_for_a_late_reader` 发图后再连发
40 条文字、期间不读广播，断言没有 `Lagged`，图片最后一条更新仍是 100% 之后的已发送。
## matrix-sdk-ui 0.19.1（Apache-2.0，`matrix-sdk-ui/LICENSE`）

crates.io 包原样导入，sha256 `e43a190b56df060b5a901d0e783e04002612981d1fef9562a44bf69330bb5257`，
与导入前 `Cargo.lock` 的 checksum 一致。上游提交见 `matrix-sdk-ui/.cargo_vcs_info.json`；包里缺的 LICENSE
从该提交 `b18166c68bb958a21f0bca8b2d8320cb53583362` 取来。原样导入和补丁分别提交。

`TimelineBuilder::build` 保留 `RoomEventCache::subscribe` 返回的快照，交给 live focus 初始化，不再稍后重读缓存。
快照和增量订阅必须取自同一边界：分开取时，中间到达的翻页/解密增量既在新快照里，又在订阅队列里，重放后会重复事件，
随后按缓存下标来的解密更新会改错条目。不能在 Wonton 渲染层去重：那会掩盖被覆盖的消息，也修不好 SDK 的事件下标。
行为回归见 `crates/wonton-matrix/src/chat_room/bootstrap_tests.rs`，用真实 Megolm 密文、重叠翻页和迟到房间密钥验证，
包含 SQLite 持久化后重建客户端的场景。

核对上游时，[crates.io](https://crates.io/api/v1/crates/matrix-sdk-ui) 的最新发布版仍为 `0.19.1`；其

`CHANGELOG.md` 未记录此修复，`TimelineBuilder::build` 丢弃订阅快照，live `init_focus` 另调 `events()`。
上游未发布的 [e3a21fd8](https://github.com/matrix-org/matrix-rust-sdk/commit/e3a21fd842a6e653f366b8cf51cb9f0bd17a3f70)
已修相同问题（`changelog.d/7284.fixed.md`），本补丁回移同一快照交接，不追随未发布的整套 SDK API 改动。
等包含该修复的版本发布后，整套 SDK 同版升级并核验下述回归；通过即可删除 SDK-ui vendor、exclude 和 patch 项。

### 缓存重置时没有条目的远程事件也清掉

`src/timeline/controller/state_transaction.rs` 的 `clear()` 在时间线里有本地回显时，只经时间线条目删远程事件。
没有条目的隐藏事件（例如主人自己的成员事件）留在 `all_remote_events` 里，条目位置和缓存位置从此错位一格；
发送成功后的回显去重按缓存下标发 `Remove{1}`，删掉的是别人的消息，自己的消息出现两次，而且不会自己恢复。
清空时把没有条目的远程事件一起删掉（倒着删，删除不动还没访问的下标）。

为什么：隧道断开期间主人发的消息先有本地回显，同步恢复时缓存整块重置（`initial: true, limited: true`）就走到上面这条
分支；`chat::resume::the_chat_recovers_when_the_tunnel_comes_back` 偶发失败（#25）时抓到的终态正是这个。
回归用假服务端驱动真实时间线，见 `crates/wonton-matrix/src/chat_room/bootstrap_tests/reset_echo.rs`。

只清孤儿还不够：有的失败里时间线根本没收到 `Clear`（`清空原因 []`），错位来自别处。`VectorDiff::Set` 带着事件，
`VectorDiff::Remove` 只带下标，所以在 `Set` 的处理里按事件 ID 核对缓存的条目位置：对不上就按 ID 找回条目；
按 ID 找到了但它是没有条目的隐藏事件就不动（map 里已经有一份），只有完全找不到才按缓存位置补一条
（插入前夹到 map 长度内，`VecDeque::insert` 越界会 panic）。`Remove` 仍在错位时按位置删，
因此清空侧的对齐是前提。

缓存还会把同一条事件再投一遍（解密前和密钥到达后的 UTD 版本各来一次 `Append`）：`Append`/`PushBack`/`Insert`
先按事件 ID 找已有条目，有就按更新处理，不再插第二条（否则时间线里同一条出现两次，读回执也认不出条目）。

## 升级

1. 先查新版 SDK 是否同时含 SDK-ui 的快照交接修复、清空时间线时清掉没有条目的远程事件、`Set` 按事件 ID 核对位置、
   matrix-sdk 的公开等价取密钥方法、网络等待期间启用备份后状态不被旧检查覆盖的修复、LatestEvents 后台任务不持有
   `Client` 的修复（连同 `Client::downgrade`/`WeakClient` 公开导出），以及上传进度节流。
   各自已有上游实现时，迁移调用并删除对应 vendor 目录、workspace exclude 和 patch 项。
2. 没有：根 `Cargo.toml` 里 matrix-sdk 系的版本一起改，`cargo fetch` 后从
   `~/.cargo/registry/cache/*/matrix-sdk-<版本>.crate` 解包，核对 sha256 和 `Cargo.lock` 的 checksum 一致，整个目录替换
   `vendor/matrix-sdk`，再从 `.cargo_vcs_info.json` 记的上游提交取 `LICENSE`。这一步单独提交，提交信息写版本和校验和。
3. 重打仍需要的补丁；matrix-sdk-ui 也按同样方式核对包摘要、导入、取 LICENSE。清空时间线时把没有条目的远程事件一并删掉；
   `Set` 处理按事件 ID 核对位置、对不上时按 ID 找回、仍没有就按位置补条目；`Append`/`PushBack`/`Insert` 按 ID 去重。
   未含快照修复时，
   把 `subscribe()` 返回的事件传给 live `init_focus`，不用后续 `events()` 重读；其他 focus 保持上游行为。
   运行 `cargo test -p wonton-matrix --lib chat_room::bootstrap_tests`，核验重叠翻页、隐藏事件和持久化重启都不重不漏、
   迟到密钥解开唯一消息。包含修复的新版通过这些回归后删除 SDK-ui 补丁。
4. `Cargo.lock` 里 `matrix-sdk`、`matrix-sdk-ui` 没有 `source`、`checksum`；其他 `matrix-sdk-*` 仍来自 crates.io，版本一致。
