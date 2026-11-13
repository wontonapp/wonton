## 默认收起的技术详情。原因、错误码和上游原文保持不变。
## 不限长度。参数是诊断数据，不是翻译键。

# CoreError 前缀。$reason 是原始错误详情。
core-error-storage = 本机存储读写失败：{ $reason }
core-error-not-ready = 尚未就绪：{ $reason }
core-error-transport = 连不上家里的服务器：{ $reason }
core-error-unauthorized = 家里的服务器拒绝了请求：{ $reason }
core-error-forbidden = 家里的服务器不让做这件事：{ $reason }
core-error-upstream = 家里的服务器出错：{ $reason }
core-error-server-too-old = 家里的服务器太旧，还做不了这件事。升级服务器后再试。

## 插件模块的技术详情，默认收起在错误、应用任务和出网代理、镜像加速设置页的“详情”里。
## 不限长度。参数是诊断数据，不是翻译键。
# 插件管理接口（crates/wonton-client/src/plugins/api.rs）。
# $reason 是原始错误详情。
plugins-api-invalid-proxy = 本地代理地址不合法：{ $reason }

plugins-api-http-client = 建 HTTP 客户端失败：{ $reason }

# $reason 是 JSON 解析器的原始错误。
plugins-api-invalid-response = 家里的服务器给的内容格式不对：{ $reason }

# $reason 是 JSON 序列化器的原始错误。
plugins-api-invalid-request = 请求体写不出来：{ $reason }

# $reason 是被拒绝的图片地址，或 URL 解析器的原始错误。
plugins-api-not-image-url = 不是插件图片的地址：{ $reason }

# $status 是原始 HTTP 状态码。
plugins-api-server-status = 家里的服务器回了 { $status }

plugins-api-not-connected = 连上家里的服务器后再试。

plugins-api-server-too-old = 家里的服务器 { $version } 太旧：这个功能要 { $minimum } 起。

# 安装、升级、卸载任务（crates/wonton-client/src/plugins/tasks.rs）。
plugins-tasks-uncertain = 连接断了，等不到家里服务器的结果，看一下已安装列表。

plugins-tasks-server-changed = 换了家里的服务器，等不到原来那台的结果。

plugins-tasks-preview-no-source = 这份预览没有来源，装不了。

# $id 是原样的任务 ID。
plugins-tasks-no-such-task = 没有这个任务：{ $id }

plugins-tasks-task-not-done = 任务还没做完。

# $title 是应用标题，按商店里的原样显示。
plugins-tasks-task-running = { $title } 正在安装、升级或卸载。

# 出网代理设置（crates/wonton-client/src/plugins/egress.rs）。
plugins-egress-save-in-progress = 上一次改动还没做完。

plugins-egress-not-loaded = 还没取到出网代理的设置。

plugins-egress-no-confirmation = 没有等待确认的改动。

plugins-egress-env-locked = 代理由 WONTON_EGRESS_PROXY 指定，不能在这里改。

plugins-egress-plugin-and-address = 使用代理插件时不能同时填写代理地址。

plugins-egress-plugin-not-installed = 还没有安装代理插件。

# $reason 是连接问题的原始详情。
plugins-egress-save-uncertain = 不确定改好了没有，连上后再看一下：{ $reason }

# 镜像加速设置（crates/wonton-client/src/plugins/registry_mirror.rs）。
plugins-registry-save-in-progress = 上一次镜像加速改动还没做完。

plugins-registry-not-loaded = 还没取到镜像加速设置。

# $reason 是连接问题的原始详情。
plugins-registry-save-uncertain = 不确定改好了没有，连上后重新读取设置：{ $reason }

matrix-login-not-current = 这次登录已经结束，或者已经换成了新的一次。

matrix-login-wrong-step = 这次登录不在这一步。

matrix-login-method-not-offered = 这台服务器不提供这种登录方式。

matrix-login-device-name-required = 请填这台设备的名字。

matrix-login-invalid-user-id = 请填完整的 Matrix ID，比如 @name:matrix.org。

matrix-login-password-required = 请填密码。

matrix-login-redirect-not-allowed = App 收不到发往这个地址的登录回调。

# 启动家里的应用（crates/wonton-client/src/plugins/start.rs）。
# $id 是原样的应用编号。
plugins-start-busy = { $id } 正在启动、安装、更新或卸载

# 家里服务器的一键更新（crates/wonton-client/src/plugins/core_upgrade.rs）：不确定阶段的说明和被拒绝的操作，显示在更新页和错误的“详情”里。
# 无参数。家里的服务器给了这个 App 不认识的结果。
plugins-core-upgrade-unrecognized = 家里的服务器给了这个 App 不认识的结果，更新 App 后再看

# 代替缺的正在运行的摘要。不是一句话。
plugins-core-upgrade-unknown-digest = 未知

# 家里的服务器报告更新完成，但正在运行的和目标对不上。$running 是正在运行的摘要或“未知”，$target 是原样的目标摘要。
plugins-core-upgrade-completed-mismatch = 家里的服务器说升级完成了，但正在运行的不是目标版本：在运行 { $running }，目标 { $target }

# 过了时限没连上，结果未知。$digest 是退回旧版本要固定的原样摘要。
plugins-core-upgrade-timed-out = 过了时限还连不上家里的服务器，不知道更新的结果。连上后这里会更新

plugins-core-upgrade-timed-out-rollback = 过了时限还连不上家里的服务器，不知道更新的结果。连上后这里会更新。要退回旧版本，在主机上再执行一次安装命令，加上 WONTON_CORE_PIN={ $digest }

plugins-core-upgrade-invalid-action = 不是更新服务器的操作

plugins-core-upgrade-already-running = 已经在更新了

plugins-core-upgrade-not-cancellable = 现在不能取消

# $reason 是连接问题的原始详情。
plugins-core-upgrade-start-uncertain = 不确定发起了没有，连上后再看一下：{ $reason }

# 兼容检查（ADR-0085）的详情：每个原因一行，{ $raw } 是清单里的原值。客户端核心按
# App 的语言写，升级页列在新版本读不了的应用下面。
plugins-core-upgrade-compat-spec = 它按规范 { $raw } 写成，新版本不支持：升级后会被标为异常。

plugins-core-upgrade-compat-feature = 它的清单用了新版本不认识的 { $raw }：升级后会被标为异常。

plugins-core-upgrade-compat-capability = 它依赖能力 { $raw }，新版本不提供：升级后会被标为异常。

plugins-core-upgrade-compat-other = 新版本读不了这个应用：升级后会被标为异常。

plugins-core-upgrade-compat-other-raw = 新版本读不了这个应用的 { $raw }：升级后会被标为异常。

## 要跨源隔离的插件网页在系统浏览器里打开（ADR-0033）。后三条显示在浏览器里。
browser-front-failed = 给浏览器用的本机地址开不了：{ $error }

browser-page-not-isolated = 这个页面不在浏览器里打开。

browser-not-paired = 云吞还没和家里的服务器配对。

browser-unknown-address = 这个地址已经失效。回到云吞，重新打开这个应用。

browser-not-connected = 云吞现在没连上家里的服务器。回到云吞，等连上后再刷新这个页面。

browser-bad-request = 请从云吞里打开这个页面。

# App 锁的设置（crates/wonton-client/src/app_lock.rs，ADR-0102）。
# $path 是设置文件的路径，$reason 是原始的 IO 错误。
app-lock-write-failed = 写不了 App 锁设置文件 { $path }：{ $reason }
app-lock-read-failed = 读不了 App 锁设置文件 { $path }：{ $reason }
app-lock-corrupt = App 锁设置文件 { $path } 坏了：{ $reason }
pages-storage-problem = 我的应用存不下，下次启动可能恢复旧状态：{ $error }
pages-file-unreadable = 这台设备上的我的应用读不了，先用默认值：{ $reason }
