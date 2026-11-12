# vendor

带补丁的第三方 crate。根 `Cargo.toml` 的 `[patch.crates-io]` 指向这里，`[workspace]` 把它们排除在外：不跑 fmt、clippy
和我们的测试，行数门禁也跳过 `vendor/`（上游的文件不归我们的拆分规则管）。

## matrix-sdk 0.19.1（Apache-2.0，`matrix-sdk/LICENSE`）

crates.io 上的包原样导入：sha256 `2afc652a144822f6fbe24084e08bc59cb483fd0b504396d492131dec0749396e`，上游提交见
`matrix-sdk/.cargo_vcs_info.json`。包里没有 LICENSE，从上游同一提交取来。

补丁两处。

第一处：`src/encryption/mod.rs` 加了公开方法 `Encryption::query_missing_secrets_from_other_sessions`，转调
matrix-sdk-crypto 已经公开的 `OlmMachine::query_missing_secrets_from_other_sessions`，向自己的其他设备再要一次缺的秘密，
再调 SDK 内部的 `send_outgoing_requests` 当场发出。只排队不发的话，请求要等下一次同步才发，可能是 30 秒的长轮询，
取回密钥的期限就过了。

为什么：自我验证时，新设备收到对方的 MAC 和 `done` 就同时发自己的 `done` 和 `m.secret.request`；要秘密的请求单独一轮同步先到旧设备时，
旧设备还没把新设备记成已验证，回 “device isn't trusted” 直接丢掉，不排队也不补发，新设备就一直缺那个秘密。
实测和时序见 [ADR-0046](../docs/adr/0046-device-verification.md)“取回密钥”一节。matrix-sdk 只在 `testing` feature 下
暴露 `OlmMachine`，正式代码没法补要；`base_client()` 绕过存储配置和跨进程锁，不能用。

第二处：`src/sliding_sync/mod.rs` 的 `generate_sync_request` 加几条 `debug!`，记下从拿到 `pos` 到发出请求之间每一步
（读回存下的位置、取 Olm 机器、读房间订阅）。这一段原本没有任何日志，同步卡在起步时只能看见前后两个时间点，分不出卡在哪，
补上才能用 `WONTON_TEST_LOG` 定位。

## matrix-sdk-ui 0.19.1（Apache-2.0，`matrix-sdk-ui/LICENSE`）

crates.io 包原样导入，sha256 `e43a190b56df060b5a901d0e783e04002612981d1fef9562a44bf69330bb5257`，
与导入前 `Cargo.lock` 的 checksum 一致。上游提交见 `matrix-sdk-ui/.cargo_vcs_info.json`；包里缺的 LICENSE
从该提交 `b18166c68bb958a21f0bca8b2d8320cb53583362` 取来。原样导入和补丁分别提交。

`TimelineBuilder::build` 保留 `RoomEventCache::subscribe` 返回的快照，交给 live focus 初始化，不再稍后重读缓存。
快照和增量订阅必须取自同一边界：分开取时，中间到达的翻页/解密增量既在新快照里，又在订阅队列里，重放后会重复事件，
随后按缓存下标来的解密更新会改错条目。不能在 Wonton 渲染层去重：那会掩盖被覆盖的消息，也修不好 SDK 的事件下标。
行为回归见 `crates/wonton-matrix/src/chat_room/bootstrap_tests.rs`，用真实 Megolm 密文、重叠翻页和迟到房间密钥验证，
包含 SQLite 持久化后重建客户端的场景。

核查上游时，[crates.io](https://crates.io/api/v1/crates/matrix-sdk-ui) 的最新发布版仍为 `0.19.1`；其
`CHANGELOG.md` 未记录此修复，`TimelineBuilder::build` 丢弃订阅快照，live `init_focus` 另调 `events()`。
上游未发布的 [e3a21fd8](https://github.com/matrix-org/matrix-rust-sdk/commit/e3a21fd842a6e653f366b8cf51cb9f0bd17a3f70)
已修相同问题（`changelog.d/7284.fixed.md`），本补丁回移同一快照交接，不追随未发布的整套 SDK API 改动。
等包含该修复的版本发布后，整套 SDK 同版升级并核验下述回归；通过即可删除 SDK-ui vendor、exclude 和 patch 项。

## 升级

1. 先查新版 SDK 是否同时含 SDK-ui 的快照交接修复和 matrix-sdk 的公开等价取密钥方法。
   各自已有上游实现时，迁移调用并删除对应 vendor 目录、workspace exclude 和 patch 项。
2. 没有：根 `Cargo.toml` 里 matrix-sdk 系的版本一起改，`cargo fetch` 后从
   `~/.cargo/registry/cache/*/matrix-sdk-<版本>.crate` 解包，核对 sha256 和 `Cargo.lock` 的 checksum 一致，整个目录替换
   `vendor/matrix-sdk`，再从 `.cargo_vcs_info.json` 记的上游提交取 `LICENSE`。这一步单独提交，提交信息写版本和校验和。
3. 重打仍需要的补丁；matrix-sdk-ui 也按同样方式核对包摘要、导入、取 LICENSE。未含快照修复时，
   把 `subscribe()` 返回的事件传给 live `init_focus`，不用后续 `events()` 重读；其他 focus 保持上游行为。
   运行 `cargo test -p wonton-matrix --lib chat_room::bootstrap_tests`，核验重叠翻页、隐藏事件和持久化重启都不重不漏、
   迟到密钥解开唯一消息。包含修复的新版通过这些回归后删除 SDK-ui 补丁。
4. `Cargo.lock` 里 `matrix-sdk`、`matrix-sdk-ui` 没有 `source`、`checksum`；其他 `matrix-sdk-*` 仍来自 crates.io，版本一致。
