# wonton-acp-bridge

把 ACP Agent 接进 Wonton 的 Matrix 桥：一个 Bot 一对容器里的 `bridge` 服务（ADR-0035）。桥复用
[`wonton-matrix`](../wonton-matrix) 的 Sliding Sync、设备密钥和交叉签名，把主人在加密私聊里发的
文字转成 ACP 会话，Agent 的回答再发回房间。

Bot 的登录状态、crypto store（`bridge/matrix/` 下的 SQLite）和房间→会话映射都在
`WONTON_BOT_DATA/bridge`，只挂给 bridge 容器（ADR-0035）。

## 排错

### Bot 说“读不了你发的加密消息”，健康显示异常

现象：桥在房间里发一条“我读不了你发的加密消息……”，插件状态变成异常（原因同那句话）。

原因：`bridge/matrix/` 里的 crypto store 丢了（数据卷被删、容器重建没挂卷）。Bot 账号没有开密钥
备份，这份 store 是它设备密钥和房间密钥的唯一一份；桥重启后用同一个设备 ID 建了全新的加密身份，
服务器收下了新密钥，但别的设备跟它建 Olm 会话用的还是旧身份签的一次性密钥，建不起来就把房间密钥
扣下不发（实测 Tuwunel v1.9 是 `m.room_key.withheld`/`m.no_olm`）。Bot 解不开收到的加密消息；桥先
当“密钥晚到”等，超过 `WONTON_ACP_UNDECRYPTABLE_SECONDS`（默认 60 秒）还没解开，就在房间里说明并
给健康检查写上原因，同一个房间 10 分钟内只说一次。

怎么恢复：重装插件并保留数据（开发环境：`deploy/dev/core/dev.sh uninstall <插件 ID>` 再 `install`，
不带 `--purge`）。核心吊销旧设备、代 Bot 登录一台新设备，主人的客户端会把房间密钥发给新设备，收发
恢复；Bot 丢失期间的消息永远解不开（没有备份可取）。

### 插件起不来或一直不健康，桥日志报存储错误

现象：桥反复重启，日志里是打不开 SQLite 存储的报错。

原因：store 坏了（SQLite 损坏，比如磁盘写满时被截断）。恢复会话打不开存储就退出，插件不健康——
如实报错，不伪装成新设备从头开始。

怎么恢复：同上，重装插件并保留数据。新设备的存储目录是新建的，损坏的旧目录会被清掉。
