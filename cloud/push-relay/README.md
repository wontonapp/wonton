# 推送中继

ADR-0003 的 M2：家里服务器按 Web Push（RFC 8030）把**已经加密给设备**的通知发到这里（RFC 8291 的
aes128gcm），这里把它交给 [gorush](https://github.com/appleboy/gorush)，由 gorush 发到 APNs 或 FCM。
设备本机解开密文：iOS 是通知扩展，Android 是 App 的推送服务。

中继看到的只有：设备令牌、密文、大小和时间。看不到房间、发送者和正文；也不存、不重发，重启后
设备重新登记即可。

推送地址由 App 用设备令牌换：`POST /v1/apns {"token": …}` 或 `POST /v1/fcm {"token": …}` 回一个
`/push/<令牌>` 的 Web Push 地址。FCM 走 data 消息（`data.wonton`），由 App 自己弹通知。

## 链路

```
Tuwunel ──pushkey──▶ 核心（加密给设备的公钥）
                        │  POST https://push.808cbd.com/push/<设备令牌>
                        │  Authorization: vapid t=<JWT>, k=<公钥>
                        ▼
                     nginx（只反代回环）
                        ▼
                     relay.py   校验 VAPID、按公钥限速
                        ▼
                     gorush（127.0.0.1:8088，无鉴权，只听回环）
                        ▼
                     APNs（通知扩展）/ FCM（App 推送服务） ──解密──▶ 通知
```

包装器把 Web Push 的请求头和正文翻成 gorush 的 `/api/push`：

| Web Push             | APNs                                        |
| -------------------- | ------------------------------------------- |
| 正文（aes128gcm）    | APNs：`data.wonton`（不带填充的 base64url）；FCM 同字段 |
| `Urgency: high`      | `apns-priority: 10`，其余 `5`                |
| `TTL: n`             | `apns-expiration: now+n`，`0` 原样为 0，缺省 86400 |
| `Topic: t`           | `collapse_id`（APNs，截到 64 字节）；gorush 1.22 的 Android 路径还不支持折叠 |
| 固定                 | `apns-push-type: alert`、`mutable-content: 1`、`content-available: 1`；`aps.alert.body` 是兜底正文（配置里的 `alert`），扩展解出后覆盖 |

正文上限 2816 字节：APNs 一条 4 KiB，外壳约 200 字节，base64 又把密文撑大 4/3，再大就撞 APNs 的上限。

FCM 的请求不带 `message`：data 消息在后台也进 App 的 `onMessageReceived`，由 App 解出正文再弹通知。

响应码：`201` 收下；`410` 设备令牌失效（APNs 的 `BadDeviceToken`、`Unregistered`、`DeviceTokenNotForTopic`，
FCM 的 `Unregistered`、`NotRegistered`、`InvalidRegistration`），核心据此删登记；`502` gorush 或 APNs 出错；`503` 上游忙；`429` 限速；
`401` VAPID 不对；`413` 正文过大；`415` 不是 `aes128gcm`；`400` 令牌形状不对。

## FCM 配置

FCM 用 Firebase 的**服务账号**（`gorush.yml` 的 `android.key_path`），不是旧版服务器密钥：

1. Firebase 控制台建项目，项目设置 → 服务账号 → 生成新的私钥，下载 JSON。
2. 把 JSON 放到 `/etc/wonton-push-relay/fcm-service-account.json`，属主 `root:wonton-push`、`0640`。
3. 把 `gorush.yml` 的 `android.enabled` 改成 `true`，重启 `wonton-push-gorush`。
4. App 侧把 `google-services` 的项目号、应用 ID、API key、sender ID 和 `wonton.pushRelay` 经构建参数编进去（见
   `apps/compose/androidApp/build.gradle.kts`）；缺任何一项时 `FcmPush.configured` 为假，退回 UnifiedPush。

默认 `android.enabled: false`：没有服务账号时打开它，gorush 会在启动时 Fatal，连 APNs 一起停。

## 鉴权

RFC 8292 的 VAPID：JWT 必须是 ES256，`aud` 必须是本服务的 `origin`，`exp` 不能过期（时钟偏差 60 秒），
签名用的公钥必须登记在 `/etc/wonton-push-relay/servers.d/` 里。没登记的公钥一律 `401`。

登记（在家里的服务器上生成钥匙，只把公钥带过来）：

```bash
# 家里服务器：生成私钥（文件里就是 WONTON_PUSH_VAPID_KEY 的内容），把打印出来的公钥记下
python3 vapid-keygen.py /etc/wonton/push-vapid.key

# 中继上：登记这台，120 条/分、突发 60
bash server-add.sh my-home <公钥> 120 60
```

家里服务器的核心设 `WONTON_PUSH_VAPID_KEY`（私钥内容，一行）或 `WONTON_PUSH_VAPID_KEY_FILE`（文件路径），
再加 `WONTON_PUSH_VAPID_SUBJECT=mailto:ops@example.com` 后才会带 VAPID 头；两者只能设一个，和 `sub` 必须成对，
缺一个核心启动就报错。`deploy/install/.env.example` 有现成的两行。

## 限速

按登记的公钥令牌桶，默认 **120 条/分、突发 60**；全机 **600 条/分、突发 200**。依据：

- 一条消息一次推送，Tuwunel 会按推送规则合并，正常聊天很难持续到 2 次/秒。
- 一个家庭几台设备、对话最密的时候约 30 条/分 × 5 台 = 150 次/分，60 的突发覆盖十几秒的高峰。
- 2 核机器上 gorush 发 APNs 大约 20–50 ms/条，全机 10 次/秒留了一半余量。
- 失控的循环每台最多 2 次/秒，挡住它用满全机额度。

超过回 `429` 和 `Retry-After`。

## 假 APNs 密钥

还没有主人的 `.p8` 时，安装脚本生成一把本地 P-256 假钥匙：gorush 能起、服务能验收，真发到 APNs 会回
`InvalidProviderToken`（包装器把它归成 `502`）。`/etc/wonton-push-relay/apns.p8.is-fake` 标着这层。

拿到真钥匙之后：

1. 把 `.p8` 放到 `/etc/wonton-push-relay/apns.p8`，`chown root:wonton-push`、`chmod 0640`，删掉 `.is-fake`。
2. 改 `/etc/wonton-push-relay/gorush.yml` 的 `ios.key_id` 和 `ios.team_id` 为 Apple 后台的真实值。
3. 真机如果是 TestFlight/App Store 包，`ios.production` 改 `true`；Xcode 直装的调试包保持 `false`。
4. `systemctl restart wonton-push-gorush`，再按下面的验收发一条真的。
5. 装上 App 后它拿到的地址才有效——中继不登记令牌，只按 URL 里的令牌发。

## 部署

Ubuntu，root，不用 Docker：

```bash
bash install.sh                       # 起服务、装 vhost、签 push.808cbd.com 的证书
bash install.sh --no-cert             # 只装服务
bash install.sh --offline-binary /tmp/gorush --no-cert   # 离线
```

脚本可重复跑：已有的配置、密钥、证书都不覆盖，已经下过同版本的 gorush 也不重下。nginx 的 vhost 只写
`server_name push.808cbd.com`，每次都先 `nginx -t` 再 reload。

certbot 按 `server_name` 找块时会被主站 `808cbd` 的 `*.808cbd.com` 骗到，把推送证书装到主站块上；脚本装完证书后
把主站块的两行 `ssl_certificate` 换回 `808cbd.com` 的证书，再装自己的 vhost。

文件：

| 路径 | 内容 |
| --- | --- |
| `/opt/wonton-push-relay/relay.py` | 包装器 |
| `/opt/wonton-push-relay/gorush` | gorush 1.22.0（下载时校验 sha256） |
| `/etc/wonton-push-relay/relay.json` | 监听、上游、`origin`、兜底正文、上限 |
| `/etc/wonton-push-relay/gorush.yml` | gorush 配置（只听回环，日志不落令牌和正文） |
| `/etc/wonton-push-relay/apns.p8` | APNs 钥匙，`0640 root:wonton-push` |
| `/etc/wonton-push-relay/servers.d/*.json` | 登记的服务器和服务端限速 |
| `/var/lib/wonton-push-relay/` | 服务用户的家目录，不存东西 |
| `/etc/nginx/sites-available/push.808cbd.com` | vhost |

两个单元都以 `wonton-push` 用户跑、只监听回环、`ProtectSystem=strict`，崩了 2 秒后重起，开机自启。

## 验收

```bash
systemctl is-active wonton-push-relay wonton-push-gorush      # active
curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8790/healthz   # 200
curl -sS -o /dev/null -w '%{http_code}\n' https://push.808cbd.com/push/$(printf 'ab%.0s' {1..32})  # 401
journalctl -u wonton-push-relay -n 20                          # 一行一条请求
```

签名请求的写法见 `verify.sh`（要一台登记过的服务器，或者用 `vapid-keygen.py` 现造一把再
`server-add.sh`）。

## 排错

- `401 VAPID: this key is not registered`：`servers.d` 里没有这把公钥，或者服务还没重读（`systemctl reload
  wonton-push-relay`，换钥匙用 `server-add.sh` 会自己重读）。
- `401 VAPID: JWT 的 aud 不是 …`：家里服务器填的中继地址和 `relay.json` 的 `origin` 不一致。
- 发出去 `502 InvalidProviderToken`：假钥匙，或者 `key_id`/`team_id` 填错、`production` 和包的签名方式不匹配。
- `502 DeviceTokenNotForTopic`：`apns_topic` 和 App 的 bundle id 不一致。
- gorush 日志看 `journalctl -u wonton-push-gorush`，令牌和正文默认隐去。
