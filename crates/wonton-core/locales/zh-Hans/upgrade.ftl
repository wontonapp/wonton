## 家里的服务器自己更新的文字（ADR-0025）：App 的更新页显示的内容和详情里的原因。
## 后台检查和更新器写文字用主人最近的语言；管理接口按请求的语言回答。
## 简体中文沿用代码里现有的说法；对用户说“家里的服务器”，不说“核心”；“升级”对用户叫“更新”。

# 更新接口拒绝请求。
upgrade-not-installed = 家里的服务器不是正式安装的，不能一键更新
upgrade-not-in-container = 家里的服务器不是在容器里运行的，不能一键更新
upgrade-busy = 已经在更新了
upgrade-not-downloaded = 新版本还没下载好
upgrade-channel-changed = 渠道上的版本变了，要重新确认
upgrade-pinned = 这次安装固定在一个版本上，不跟渠道
upgrade-not-upgrading = 现在没有在更新
upgrade-cancel-too-late = 更新已经开始改动，不能取消了
upgrade-launch-updater-failed = 启动更新器失败：{ $error }

# 接口自己查不到要答的东西。
upgrade-container-missing = 查不到家里的服务器的容器 { $name }
upgrade-image-missing = 查不到正在运行的镜像 { $image }

# 后台检查（每 6 小时一次，打开更新页也会查）。
upgrade-check-core-failed = 查家里的服务器的容器失败：{ $error }
upgrade-latest-version-failed = 查不到 { $source } 的最新版本：{ $error }
upgrade-image-lookup-failed = 查镜像失败：{ $error }
upgrade-registry-digest-missing = 镜像仓库没有返回 { $source } 的摘要
upgrade-free-space-failed = 查 { $path } 的磁盘空间失败：{ $error }
upgrade-pull-stalled = 拉 { $image } 连续 5 分钟没有进展
upgrade-auto-download-skipped = 磁盘剩不到 5 GB，没有自动下载
upgrade-download-no-space = 磁盘不够：下载大约要 { $needed } 字节，只有 { $free } 字节可用
upgrade-download-failed = 下载不了：{ $error }
upgrade-image-missing-after-pull = 下载完了，本机还是没有 { $reference }

# 更新器容器把自己做的事写进状态文件，用主人的语言。
upgrade-updater-crashed = 更新器反复崩溃
upgrade-phase-unreadable = 读不出状态文件里的阶段
upgrade-updater-gone = 更新器不在了，不确定 { $phase } 这一步有没有做完
upgrade-rollback-cause = { $message }。回滚的起因：{ $cause }
upgrade-restore-also-failed = { $message }。之后启动回原来的版本也失败了：{ $error }
upgrade-state-unreadable = 读不出 { $path }，可能是新版本写的
upgrade-state-no-reason = 状态文件里没有写原因
upgrade-state-serialize = 序列化更新状态失败
upgrade-no-backup-dir = 状态文件里没有备份目录
upgrade-restore-pin-hint = 要退回旧版本，在主机上再执行一次安装命令，加上 WONTON_CORE_PIN={ $digest }

# 在主机上拉镜像、跑容器。
upgrade-list-images-failed = 新镜像 { $image } 列不出它要用的镜像：{ $error }
upgrade-no-image = 本机没有镜像 { $image }
upgrade-inspect-image-failed = 查镜像 { $image } 失败：{ $error }
upgrade-pull-image-failed = 拉不到镜像 { $image }：{ $error }
upgrade-pull-image-timed-out = 拉镜像 { $image } 超过了时限
upgrade-run-in-image-failed = 在镜像 { $image } 里执行 { $cmd } 失败：{ $error }
upgrade-window-exceeded = 从停掉家里的服务器到新版本起来，超过了 30 分钟
upgrade-components-failed =
    { $target ->
        [new] 处理新版本不支持的组件失败：{ $error }
       *[old] 处理旧版本不支持的组件失败：{ $error }
    }
upgrade-up-failed = 服务没有起来：{ $error }
upgrade-core-image-wrong = 家里的服务器的容器用的是 { $actual }，不是 { $image }
upgrade-core-container-missing = 没有家里的服务器的容器
upgrade-containers-failed = 查系统服务的容器失败：{ $error }
upgrade-observe-timed-out = 观察阶段超过了时限
upgrade-rollback-timed-out = 回滚超过了 20 分钟
upgrade-stop-service-failed = 停服务 { $service } 失败：{ $error }
upgrade-stop-core-failed = 停家里的服务器失败：{ $error }

# 备份和恢复（哪一步出错写在状态文件里）。
upgrade-backup-failed = 备份失败：{ $error }
upgrade-backup-incomplete = 备份 { $path } 没有写完，不能用来恢复
upgrade-volume-size-failed = 查卷 { $volume } 的大小失败：{ $error }
upgrade-volume-size-unknown = 查不出卷 { $volume } 的大小：{ $error }
upgrade-disk-space-failed = 查磁盘空间失败：{ $error }
upgrade-backup-no-space = 磁盘不够：备份要 { $needed } 字节，只有 { $free } 字节可用，旧备份能腾出 { $old } 字节
upgrade-backup-volume-failed = 备份卷 { $volume } 失败：{ $error }
upgrade-restore-failed = 恢复备份失败：{ $error }
upgrade-init-failed = 新版本的 init 失败了：{ $error }

# 切换之后盯着各服务时发现的问题。
upgrade-service-container-gone = 服务 { $service } 的容器不见了
upgrade-service-container-replaced = 服务 { $service } 的容器被换掉了
upgrade-service-image-changed = 服务 { $service } 换了镜像：{ $image }
upgrade-service-restarted = 服务 { $service } 重启了
upgrade-service-status = 服务 { $service } 的状态是 { $status }
upgrade-service-unhealthy = 服务 { $service } 变得不健康
upgrade-one-shot-exit-code = 一次性服务 { $service } 的退出码是 { $code }

# 系统报错前面的短说明，如“读取 /x: Permission denied”。
upgrade-env-missing-key = { $path } 里缺 { $key }

# compose（核心和更新器里）和一次性容器。
upgrade-compose-run-failed = 运行 { $path } 失败：{ $error }
upgrade-compose-timed-out = compose 超过 { $seconds } 秒还没结束
upgrade-compose-wait-failed = 等 compose 结束失败：{ $error }
upgrade-compose-failed = compose 失败（{ $status }）：{ $error }
upgrade-one-shot-timed-out = 容器 { $name } 超过 { $seconds } 秒还没结束
upgrade-exit-code-unknown = 查不到容器 { $name } 的退出码
