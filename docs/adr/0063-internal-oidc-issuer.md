# ADR-0063 签发方用插件网络里的内部名字，浏览器走的地址不变

- 状态：已接受（2026-11-08，主人定）；2026-11-13 补充重建中断约束（ADR-0105）。部分取代 ADR-0009“流程”第 4 条（插件后端经 `auth.<base>` 访问签发方）

## 背景

基础域名默认是 `wonton.localhost`。`http://*.localhost` 是浏览器认的安全上下文；隧道里只跑 http（ADR-0009）。
插件网页要用的 `crypto.subtle`、`SharedArrayBuffer`（ADR-0033）、Service Worker 和 `Sec-Fetch-*`（ADR-0011），都靠这一点才有。

签发方原来是 `http://auth.<base>:<端口>`，插件后端经核心在插件网络里的同名别名访问它。有的库按 RFC 6761 把 `*.localhost`
直接当成回环，不发 DNS 查询，也不读 `/etc/hosts`，别名对它们不起作用。实测撞上的有两个：
- Vaultwarden 用 hickory-resolver 解析签发方，OIDC discovery 失败；
- Nextcloud 用 curl 8.14 访问。

插件访问自己的公网地址 `<slug>.<base>` 是同一类问题。Ente 的服务端要访问自己签给浏览器的 S3 地址；它用 glibc 解析，
那个名字在插件网络里没有别名，报 NXDOMAIN。

## 决策

### 签发方

- Hydra 的 `urls.self.issuer` 改成 `http://auth.wonton.internal:<网关端口>`，同 `matrix.wonton.internal` 等内部别名。
  `urls.self.public` 留在 `http://auth.<base>:<端口>`：Hydra 登录、授权确认后把浏览器送回 `public`，浏览器只认这个地址。
- 申请 `sso: oidc` 的插件，在自己的网络里拿到的核心别名换成 `auth.wonton.internal`，不再有 `auth.<base>`。
  网关把这两个主机名都转给 Hydra 的公开端口。
- `WONTON_OIDC_ISSUER` 注入内部签发方。令牌里的 `iss`、发现文档的 `issuer`、插件配的签发方三者一致。

### 发现文档

Hydra v26.2.0 实测（`issuer` 设成内部名字，`public` 设成浏览器地址）：`authorization_endpoint`、`token_endpoint`、`userinfo_endpoint`
用 `public`，`jwks_uri`、`revocation_endpoint`、`end_session_endpoint` 用 `issuer`。两边各错一半，只改 Hydra 的配置做不到。

核心按谁来访问改写端点，改写只写一处：经网关返回的 `/.well-known/openid-configuration`（两个主机名都一样）和注入插件的变量都用它。

| 端点 | 谁访问 | 地址 |
|---|---|---|
| `authorization_endpoint`、`end_session_endpoint` | 浏览器 | `auth.<base>` |
| `token_endpoint`、`userinfo_endpoint`、`jwks_uri`、`revocation_endpoint` | 插件后端 | `auth.wonton.internal` |

其余字段原样转发。`WONTON_OIDC_AUTH_URL` 给浏览器地址，`WONTON_OIDC_TOKEN_URL`、`WONTON_OIDC_USERINFO_URL` 给内部地址。

### 插件访问自己的公网地址

- 有网页的插件，在自己的网络里多一个核心别名 `<slug>.<base>`。
- 网关收到发往 `<slug>.<base>` 的请求，来源是这个插件自己网络里的容器时，不要会话，直接转给它的网页服务，不代表任何人：
  - 不注入代登录的身份头；
  - 来源请求带的身份头照常剥掉；
  - 不算主人，也不算家人。

  别的来源照旧要会话。
- 只有经 DNS 解析的应用能用上（glibc、musl，如 Ente）。curl、hickory 这类直接把 `*.localhost` 当回环的应用，访问自己公网地址
  底座不解决，由插件自己绕开。基础域名留在 `.localhost`。

### 已装插件的登录

不写迁移，不保留旧签发方。核心升级后，用 SSO 的插件按新的 `WONTON_OIDC_ISSUER` 重建容器；Hydra 的 `subject`（用户 ID）不变，
只有 `iss` 变了。按应用怎么认人分三种：

- **只按 `sub` 或邮箱认人**（ADR-0022）：照常登录，用户无感。
- **按 `iss` 加 `sub` 认人**：会把同一个人当成新用户，进到一个空账号；邮箱和旧账号重了时，应用自己报错。
- **首次启动就把签发方写进自己的数据库**（不每次读变量）：discovery 对不上，应用自己报登录失败。

这两种都显示在应用自己的页面上，核心不去猜，也不替应用改说法。主人重装插件，或者在应用里重新关联账号。
官方索引里用 SSO 的插件都在升级后的真实栈上过一遍，每个属于哪种、主人要做什么，写进插件页。App 不改，照常打开插件页面。

升级触发的渲染漂移重建遵守 [ADR-0105](0105-plugin-runtime-recovery.md) 的运行意图与中断续建约束。

## 理由

- 签发方是个标识符，浏览器从不访问它本身，只走授权和登出两个端点。把这个标识符换成插件网络里能解析的名字，
  浏览器那一侧的地址、安全上下文和 Hydra 的 cookie 都不动，App 不用改。
- `.wonton.internal` 下的别名所有插件都在用，从没撞过解析问题。
- 签发方和后台地址分开是业内常见做法，如 Keycloak 的前台地址和后台地址可以分开配。
- 改写放在核心：核心本来就站在 `auth.*` 前面转发，Hydra 自己分不清哪个端点是谁在访问。
- 插件自己网络里的容器本来就能直连它的网页服务。网关不要会话、不带身份地转给它，没有给出新的访问能力，只是把端口和主机名对上了。

## 代价与风险

- 已装的、按 `iss` 认人的 SSO 插件要重新关联一次（主人知道这个代价）。
- 依赖方要求发现文档里的端点和签发方同主机，这种情况会失败。少见，靠官方插件逐个验证兜住。
- 在浏览器里直接做 OIDC 的单页应用，会到签发方地址取发现文档，而 WebView 只让基础域名走隧道代理。官方插件里还没有这类。
  遇到时，再把签发方的名字加进 App 代理的放行名单。
- 签发方的名字每台都一样，靠密钥（`jwks`）区分。插件只信任自己这台核心，用不上名字区分。
- curl、hickory 类应用访问自己公网地址的问题留着。

## 没选的做法

- **开发和正式环境都换成 `.test` 之类的普通域名**：所有插件网页丢掉安全上下文，Vaultwarden 的网页密码库、Actual 都用不了。
  隧道里也不加 HTTPS（ADR-0009）。
- **给插件容器配一个会正确应答的 DNS**：撞上的库根本不发 DNS 查询。
- **签发方用 `auth.<Matrix 服务器名>`**：开发环境的服务器名就是基础域名（ADR-0009），又会落回 `.localhost`；每台不同的名字也没有用处。
- **在插件容器的网络命名空间里放一个转发**：要改插件的 compose 结构，每个插件多一个容器，还要占它的回环端口。

## 关联

- ADR-0009（身份、配对和单点登录）、ADR-0011（安全模型）、ADR-0022（主人的身份）、ADR-0023（出网代理，`NO_PROXY` 里的签发方）、
  ADR-0033（插件标签页，跨源隔离）。
