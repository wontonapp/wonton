# ADR-0077 新设备验证：确认方式的取舍，和没有已验证设备时的出路

- 状态：已接受（2026-11-11，主人定）；“QR 先不做”由 [ADR-0087](0087-pairing-key-handoff.md) 修订
- 补充：[ADR-0046](0046-device-verification.md)、[ADR-0005](0005-native-clients-thick-core.md) 和 [ADR-0003](0003-matrix-and-encrypted-push.md) 的恢复与重置；只细化重置入口、按钮可见性和它与恢复密钥的关系，不改那里的决定

## 背景

issue #21（迁移自归档 #274）要的是“新设备由已有设备确认”，并点名两件要定的事：确认方式；账号里已经没有任何已验证设备时怎么办。做之前先写 ADR。

用最新 main 核对，issue 正文里“核心目前没有这个能力”已经过期：

- SAS 确认按 [ADR-0046](0046-device-verification.md) 实现：发起、接受、确认、拒绝、取消五个 `Action`，状态在
  `VerificationFlow`（`crates/wonton-protocol/src/verification.rs`），流程在
  `crates/wonton-client/src/chat/verification.rs`，SDK 包装在 `crates/wonton-matrix/src/verification.rs`，两端都有界面
  （`apps/ios/Wonton/DeviceVerificationView.swift`、`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/DeviceVerificationScreen.kt`）。
- 恢复密钥和重置本来就有（[ADR-0003](0003-matrix-and-encrypted-push.md) 的“端到端加密”验证结果）：重置入口在聊天记录备份页，`Action::ResetRecovery` 换新身份和新密钥。

所以本文不再重新决定 SAS，只把确认方式的公开事实和取舍写在一个地方，并决定 ADR-0046 留给“恢复密钥或重置”的那一半：没有已验证设备时的入口、步骤和后果。

配对只说明核心放行了这台设备，说明不了它是主人手里那台：核心能签主人的登录令牌，也能造一台同名设备（[ADR-0009](0009-identity-pairing-sso.md)、[ADR-0046](0046-device-verification.md)）。线下比对挡的正是核心。第二条路（恢复密钥）不经过设备间确认，靠主人手里的密钥。

## 调研：matrix-sdk 0.19.1 给了什么、没给什么

版本由 `Cargo.lock` 固定（`matrix-sdk` 0.19.1）。`matrix-sdk` 和 `matrix-sdk-ui` 在 `vendor/`；`matrix-sdk-crypto` 在 registry 的 `matrix-sdk-crypto-0.19.1/`，下文按 crate 名写路径。结论按 SDK 源码：

- **SAS**：完整。SDK 有表情、数字、双方 `confirm()`、`mismatch()`、`cancel()`（`vendor/matrix-sdk/src/encryption/verification/sas.rs:81,97,106,155,160,166`）。
- **应用内 QR**：完整，两种自我模式（显示端能签名一种、不能签名一种），有生成和扫描（`vendor/matrix-sdk/src/encryption/verification/requests.rs:172,189`、`qrcode.rs:99`）。前提：
  - `qrcode` feature 不在默认列表里（`vendor/matrix-sdk/Cargo.toml:58-63`，开关在 `:121-124`），要显式打开。
  - 扫描端要给 SDK 原始二进制载荷，不是字符串。两端现在的扫码页只取文字：iOS `barcode.payloadStringValue`
    （`apps/ios/Wonton/QRScannerView.swift:69-71`），Android ZXing 的 `.text`
    （`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/QrScanner.kt:153`）。要用，得另取原始字节并过 FFI。
  - 显示端在对方扫完后仍要主人点一次确认。
- **已验证设备“直接批准”**：没有对应协议。只有手工签名 `Device::verify()`
  （`vendor/matrix-sdk/src/encryption/identities/devices.rs:291`）：用 self-signing 私钥签那台设备，不通知对端、不建立反向信任、不触发 secret gossip。它不能替代 SAS 或 QR。
- **请求发给谁**：只发给账号里被 self-signing key 签过的其他设备（`matrix-sdk-crypto-0.19.1/src/identities/user.rs:315-329,1290-1301`）；请求 10 分钟过期（`matrix-sdk-crypto-0.19.1/src/verification/requests.rs:67`），多台设备都能接，后来接的只被动看着（被接手时是 `AcceptedElsewhere`）。有没有这样的设备由
  `has_devices_to_verify_against()` 判断（`vendor/matrix-sdk/src/encryption/mod.rs:1132-1148`）。
- **验证之后**：只有实际验证了本账号身份（own identity），SDK 才把新设备记成已验证，并自动要缺的交叉签名私钥和备份密钥
  （`matrix-sdk-crypto-0.19.1/src/verification/mod.rs:607-616,644-651,594`）；旧设备只回给同账号、已验证的设备。验证“完成”不等于历史可读，密钥没到齐前仍不能报成功（ADR-0046 的 `FetchingKeys`/`KeysMissing`）。
- **推送**：验证请求是 to-device 消息，不进房间，现有推送唤醒不了旧手机；旧手机要开着 App（ADR-0046）。
- **重置身份**：SDK 有 `Recovery::reset_identity`，要过 UIA（`vendor/matrix-sdk/src/encryption/recovery/mod.rs:438`）。Tuwunel v1.9.4 只给 `org.matrix.login.jwt` 一个阶段，核心先经隧道要一次性登录令牌，主人不用输密码（[ADR-0003](0003-matrix-and-encrypted-push.md) 验证结果；`crates/wonton-matrix/tests/tuwunel/encryption.rs` 里的重置测试）。

规范里有、SDK 没给的：SAS 的表情表、数字、双方的 `m.key.verification.mac`/`done`，QR 的 `m.qr_code.show.v1`/`m.qr_code.scan.v1`/`m.reciprocate.v1`、并发和取消，规范和 SDK 都有；SDK 缺的只有扫描端的原始载荷提取（属于我们的壳，不属于协议）和默认关着的 `qrcode` feature。规范里没有“直接批准”这种方法：`Device::verify()` 是交叉签名 API，不是验证事件。更外面的 MSC4108（扫码登录并转交秘密）规范有、SDK 也有 OAuth 模块，但 Tuwunel 和 JWT 登录接不上，ADR-0046 已经拒绝过一次。

SDK 给的是密码学、消息和秘密交换；没给的是流程、入口、失败后的出路，和“一台可信设备都不在”时的退化路径。这些是本文要定的。

## 决策

### 确认方式：保持 SAS，QR 先不做

维持 ADR-0046 的 SAS，理由不变：标准流程，SDK 已经写好，别的 Matrix 客户端也能互验，两端各点一次“相同”就完成线下比对。

三者的代价（从新设备已登录、两端 App 已打开算）：

| 方式 | 主人动作 | 挡不挡核心 | 主要成本 |
| --- | --- | --- | --- |
| SAS | 发起、接受、两端各确认一次，对照 7 个表情或 3 组数字 | 挡 | 4 次点击，看得懂的表情表 |
| QR（显示端能签名） | 发起、接受、扫一次、显示端确认 | 挡 | 3 次点击加 1 次扫描；开 feature、两端保存原始载荷 |
| 直接批准 | 旧设备按一次允许 | **不挡**核心换设备 | 1 次点击 |

QR 先不做：它和 SAS 达到同样的信任和密钥结果，却要动两端的相机链路和 FFI。以后要做时按上面的 QR 行做，不只打开 feature。

### 没有已验证设备：恢复密钥优先，重置是最后一条路

**顺序**。新设备先显示聊天列表的“输入恢复密钥”提醒，进备份页；输入恢复密钥保留历史，重置丢历史。备份页在 `can_verify_with_other_device` 为假时直接给密钥输入框，不给“用另一台设备确认”。

**重置入口**。备份页底部一个次要按钮，和恢复密钥输入并列，不打断它：

- 叫“重置备份”，文字在壳（ADR-0043），本文只定这个词。
- 可见：只在缺密钥、备份残缺时出现，即 `recovery` 是 `Incomplete`，或者 `Disabled` 且本设备未验证；备份完好（`Enabled`，本设备已验证、密钥齐全）不出现。显示新密钥待保存时也隐藏（还没抄下来就作废）。
- 点了先出一个危险样式的确认框，写清换新密钥、旧密钥作废、旧消息可能读不了、其他设备要输入新密钥。
- 断网时按钮照常显示，点了报 `NotReady`，账号不动（先要登录令牌，要不到就不开始）。

**重置做什么**。删掉账号的密钥备份和秘密存储，换新的交叉签名身份，本设备成为新身份下已验证的设备，再开启备份并显示新密钥一次。之后：

- 旧恢复密钥永久作废；其他设备变成 `Incomplete`，聊天列表出现“输入恢复密钥”提醒，输入新密钥后恢复完整。不另加更强的提示：提醒本来就在该设备列表的顶部，点进去就是备份页。
- 只在旧备份里、本机也没有的消息解不开；本机手里有的房间密钥照常能解。
- 本来就有全部私钥的设备输入残缺备份的密钥也显示完整，只有没有私钥的设备看得出备份残缺。

**不自动重置**。验证失败、请求超时、对方回 `KeysMissing` 都只结束流程；只有主人点按钮才重置。备份页上的重置按钮在流程进行中也留着（对方不在时是唯一出路）；点重置时先取消进行中的流程，取消发不出去也照常重置。

### 核心和壳

- **核心**：状态和规则。确认的 `VerificationFlow`（阶段、取消类别、表情序号和符号、对端设备），操作的 `Action`，重置的 `ResetRecovery`，恢复状态和 `recovery_key`。流程里所有“能不能点”“走到哪一步”“为什么结束”都由核心算，壳不改写。
- **壳**：只画。按阶段和类别取本地化文字（ADR-0043），画确认框、输入框和按钮，发 `Action`。表情名按序号取，四种语言都要。
- **补两类规则进核心**：两端现在各写一份同样的判断——什么时候给重置按钮（`apps/ios/Wonton/ChatBackupView.swift:136-143`、`apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/ChatBackupScreen.kt:190-195`），以及恢复页给输入框、给两个选项还是没有（`apps/ios/Wonton/ChatBackupView.swift:62-84`、`ChatBackupScreen.kt:111-128` 的 `NextStep`）。由核心给出 `EncryptionStatus` 上的 `can_reset` 和 `recovery_next_step`（`EnterKey`/`ChooseDeviceOrKey`/`ShowFlow`/`Progress`/`None`），和 `can_verify_with_other_device` 并排；壳只留本地的“主人点了输入恢复密钥”这一层显示开关，按类别映射文字和按钮。单独一个小 PR 先做，只加这两个字段，不改别的协议。

### 测试

用行为测试，两台真实客户端对一次性的真实 Tuwunel（`crates/wonton-client/tests/it/chat/support.rs` 的 `Home`、`Phone`：真隧道、真核心登录流程，不起 mock）。验证流程已有的用例在 `crates/wonton-client/tests/it/chat/verification.rs`；重置只有 `wonton-matrix` 那条直连测试（`crates/wonton-matrix/tests/tuwunel/encryption.rs:157` 直接调 `reset_recovery`），客户端链路（`Action`、经核心要登录令牌、UIA、`resetting_recovery` 门、视图过渡、新密钥只显示一次）要在同一层补：

- 没有别的设备、没有密钥：列表出现输入密钥提醒，备份页给输入框和重置按钮，`can_reset` 为真。
- 重置后本设备已验证、备份完整、新密钥待保存；旧密钥在第三台设备上报 `InvalidInput`，新密钥成功。
- 重置时断网或核心拒绝：`NotReady`/`Unauthorized`，恢复状态不变，原密钥仍可用；要令牌期间状态流不把 `InProgress` 盖回去。
- 进行中的确认被重置取消，对端收到“对方取消”，之后能重新发起。
- `can_reset` 的真值表按上面“可见”一条，逐个状态断言，`Enabled`（已验证、密钥齐）为假。
- `recovery_next_step` 的每个类别各给一屏的断言（输入框、两个选项、流程、进度、无）。
- 新密钥只显示一次：确认保存前 `can_reset` 为假、再重置也换不掉它，确认后不再出现。

## 理由

- 按 issue 的两件事给出可执行的答案，而不是把已经实现的 SAS 再决定一遍。
- 重置是“手机全丢、密钥也丢”时唯一走得通的路；放在备份页、用危险确认框，是因为它的代价（旧消息和旧密钥）只有主人能接受。
- 入口跟随恢复状态，不新开页面；和其他备份操作在同一个地方。
- 备份完好时不出现：那时代替不了什么，换掉还会让其他设备重输密钥。
- `can_reset` 和 `recovery_next_step` 进核心以后，两端不会因为一处改了 switch 而给出不同的按钮或分支。

## 没选的

- **“旧设备按一下允许”（直接批准）**：少点几下，但 `Device::verify()` 只是本地交叉签名，不通知对端、不建立反向信任、不转秘密，挡不住核心换设备，规范里也没有这种方法。要做得另有一条不经核心的比对（例如两端对照由新设备公钥算出的值），否则仍挡不住核心。
- **现在做 QR**：结果和 SAS 一样，成本多一条相机链路和 FFI 原始载荷，两端扫码页都得改。
- **自动重置**：SDK 一失败就换身份会把还能救的历史一遍丢光；主人也来不及看新密钥。
- **只在完全没有恢复密钥时才给重置**：备份残缺、服务器上还有别人的旧备份、或者密钥抄错一位时同样需要重置，只看“有没有密钥”判断条件更绕。
- **核心保管恢复密钥**：ADR-0005 已经选了自己保管，本文不改。

## 代价与风险

- 重置后，没被本机解过的旧消息再也取不回；确认框和备份页状态都要如实说。
- 其他设备要各输一次新密钥，没有设备间的自动转交。
- UIA 只认登录令牌：换到不给 `org.matrix.login.jwt` 阶段的服务器或 Tuwunel 改流程，重置会报 `Upstream`，要按版本复测。
- 备份页同时放着密钥输入和重置，误点的代价靠确认框兜住，不做二次校验。备份完好时页面上没有重置，想换新身份要先让这个状态失效。
- secret gossip 送的是缺的交叉签名私钥和备份密钥（`m.megolm_backup.v1`），不含房间密钥；房间密钥靠备份按需下载，重置后从没上传过的消息不在任何备份里。

## 待验证

- 客户端核心这一层的重置没有行为测试，只有 `wonton-matrix` 对 Tuwunel 的直连测试和两端界面。上面“测试”一节是补齐清单。
- 重置开始的时机和 SDK 的 UIA 会话在隧道断开、重连时的行为只在源码上看过，没实测。
- 并发：重置和自动开启备份（`watch` 里的 `resetting_recovery` 门）只测过顺序执行。
