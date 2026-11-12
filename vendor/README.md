# vendor

带补丁的第三方 crate。根 `Cargo.toml` 的 `[patch.crates-io]` 指向这里，`[workspace]` 把它们排除在外：不跑 fmt、clippy
和我们的测试，行数门禁也跳过 `vendor/`（上游的文件不归我们的拆分规则管）。

## matrix-sdk 0.19.1（Apache-2.0，`matrix-sdk/LICENSE`）

crates.io 上的包原样导入：sha256 `2afc652a144822f6fbe24084e08bc59cb483fd0b504396d492131dec0749396e`，上游提交见
`matrix-sdk/.cargo_vcs_info.json`。包里没有 LICENSE，从上游同一提交取来。

补丁只有一处：`src/encryption/mod.rs` 加了公开方法 `Encryption::query_missing_secrets_from_other_sessions`，转调
matrix-sdk-crypto 已经公开的 `OlmMachine::query_missing_secrets_from_other_sessions`，向自己的其他设备再要一次缺的秘密，
再调 SDK 内部的 `send_outgoing_requests` 当场发出。只排队不发的话，请求要等下一次同步才发，可能是 30 秒的长轮询，
取回密钥的期限就过了。

为什么：自我验证时，新设备收到对方的 MAC 和 `done` 就同时发自己的 `done` 和 `m.secret.request`；要秘密的请求单独一轮同步先到旧设备时，
旧设备还没把新设备记成已验证，回 “device isn't trusted” 直接丢掉，不排队也不补发，新设备就一直缺那个秘密。
实测和时序见 [ADR-0046](../docs/adr/0046-device-verification.md)“取回密钥”一节。matrix-sdk 只在 `testing` feature 下
暴露 `OlmMachine`，正式代码没法补要；`base_client()` 绕过存储配置和跨进程锁，不能用。

## 升级

1. 先看新版 matrix-sdk 有没有公开等价的方法。有：删掉 `vendor/matrix-sdk` 和根 `Cargo.toml` 的 `[patch.crates-io]`，
   改用上游的方法，到此为止。
2. 没有：根 `Cargo.toml` 里 matrix-sdk 系的版本一起改，`cargo fetch` 后从
   `~/.cargo/registry/cache/*/matrix-sdk-<版本>.crate` 解包，核对 sha256 和 `Cargo.lock` 的 checksum 一致，整个目录替换
   `vendor/matrix-sdk`，再从 `.cargo_vcs_info.json` 记的上游提交取 `LICENSE`。这一步单独提交，提交信息写版本和校验和。
3. 重打上面那一处补丁，单独提交。
4. `Cargo.lock` 里只有 `matrix-sdk` 没有 `source`、`checksum`；其余 `matrix-sdk-*` 仍来自 crates.io，版本和它一致。
