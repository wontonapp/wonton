# ADR-0078 OIDC 表单回调：只开放登记端点，不授予网关身份

- 状态：已接受（2026-11-11）
- 取代：[ADR-0011](0011-security-model.md) 中“OIDC 回调只能用 `query`”的约束

## 背景

[OIDC 使用的 Form Post 响应模式](https://openid.net/specs/oauth-v2-form-post-response-mode-1_0.html#FormPostResponseMode)
要求浏览器向客户端登记的回调地址提交 `POST`，正文为 `application/x-www-form-urlencoded`。
签发方和插件不同源，普通插件请求的来源隔离会挡住这种回调；改网关会话 Cookie 的 SameSite
不能代替回调策略，也不应为此放宽其他跨源请求。

## 决策

复用 `sso: oidc` 插件的 `webui.oidc.redirect_paths`：这份路径既登记到身份提供方，也作为
该插件的回调白名单，不增加清单字段。仅同时满足下列条件的请求可以不带网关登录状态到达应用：

- 请求路径与本插件登记的回调路径精确相同，不作前缀匹配。
- 方法为 `POST`；`Content-Type` 取第一个 `;` 前的媒体类型、去两侧空白后，不分大小写精确等于
  `application/x-www-form-urlencoded`。允许 `charset` 等参数，不接受伪造的类型前缀。
- `Origin` 缺失，或不分大小写等于核心生成的签发方浏览器 origin（`auth.<基础域名>`）或本插件 origin，
  包括协议和端口。`null`、其他来源和错误端口都不符合条件；不靠 `Sec-Fetch-*` 判定此例外。

这是**有限的未认证可达面，不是认证**：网关不检查表单里的 OIDC 凭据，不创建网关身份或会话，
也不向应用注入网关凭据。可从有效插件会话或隧道身份识别出人时，仍执行插件的家人开关；
没有身份时无法按人执行此检查，不能假设先前已检查过。回调后的普通页面仍需原有网关鉴权。

通用的来源隔离规则不变；`query`/GET 导航、其他方法、登出回跳、其他路径和 `sso: proxy`、`none`
都不增加例外。转发仍剥除 `wonton_*` Cookie；响应仍去掉 `Set-Cookie` 的 `Domain`，丢弃 `wonton_*`
Cookie，不让应用设置网关会话。

## 安全责任与代价

- 非浏览器客户端能省略或伪造 `Origin`。它只是一层浏览器来源隔离，不是签发方认证。
- 应用必须把回调绑定到它发起的登录：校验受保护或不可预测的 `state` 及登录关联，并按所用流程
  校验关联 Cookie、PKCE、授权码、nonce 和令牌。网关的路径、方法和来源检查不能代替这些校验。
  `state` 的有效期和防重放保证取决于应用的完整流程，不由本例外提供。
- 例如 [Kavita 固定源码的接线](https://github.com/Kareadita/Kavita/blob/d77d956b9551227d8be2ee488b08f14aa3a341e5/Kavita.Server/Extensions/IdentityServiceExtensions.cs#L228-L258)
  使用 code flow 和 `/signin-oidc`，其 [OpenIdConnect 依赖为 10.0.11](https://github.com/Kareadita/Kavita/blob/d77d956b9551227d8be2ee488b08f14aa3a341e5/Kavita.Services/Kavita.Services.csproj)。
  该框架先[解保护 state 并检查关联](https://github.com/dotnet/aspnetcore/blob/v10.0.11/src/Security/Authentication/OpenIdConnect/src/OpenIdConnectHandler.cs#L641-L725)，
  [缺少关联 Cookie 或标记不匹配即失败](https://github.com/dotnet/aspnetcore/blob/v10.0.11/src/Security/Authentication/Core/src/RemoteAuthenticationHandler.cs#L248-L280)。
  这说明应依赖的应用校验，不代表所有应用都采用相同实现。
- 网关放行不保证完整登录可用。上述框架的[关联 Cookie](https://github.com/dotnet/aspnetcore/blob/v10.0.11/src/Security/Authentication/Core/src/RemoteAuthenticationOptions.cs)
  和 [nonce Cookie](https://github.com/dotnet/aspnetcore/blob/v10.0.11/src/Security/Authentication/OpenIdConnect/src/OpenIdConnectOptions.cs)
  默认为 `SameSite=None; Secure`。即使 `http://*.localhost` 在浏览器里可用，也不能推广到普通 HTTP
  基础域名；非 localhost 的普通 HTTP 可能无法存取这些应用 Cookie。不能为此关闭关联校验。

## 没选

- 放宽所有跨源 POST，或给整个插件免鉴权：开放面超过协议需要。
- 只让应用改用 `query`：排除上游默认采用标准 `form_post` 的应用。
- 给特定应用增加表单转 GET：核心不应改写应用协议，也不应把表单凭据搬到地址栏。
