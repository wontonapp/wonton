# 能力访问、依赖错误与安装确认。
gateway-capability-permission = 应用 { $id } 没有声明能力 { $capability }，或还没有获准使用它的提供方
plugin-capability-missing = 能力 { $capability } 缺少提供方，请先安装一个再继续
plugin-capability-stopped = 应用 { $provider } 还不能提供能力 { $capability }
plugin-capability-credential-failed = 应用 { $provider } 为能力 { $capability } 准备或吊销访问凭据失败，退出码 { $code }：{ $detail }
plugin-capability-in-use = 这些应用还需要这个提供方：{ $plugins }。请先安装其他提供方，或卸载这些应用。
plugin-capability-provider-install = 一起安装 { $provider }（{ $id }）
plugin-capability-provider-permission = { $provider } 需要这项权限：{ $permission }
plugin-capability-provider-optional-off = { $provider } 的可选权限会保持关闭
plugin-capability-connect = { $consumer } 将能经 { $provider } 使用能力 { $capability }
plugin-capability-provider-failed = 安装提供方 { $provider } 失败：{ $error }
plugin-capability-install-selection = 所选提供方和缺少的必需能力对不上，请重新打开安装预览，从列表中选择提供方。
plugin-capability-request-format = 无法读取这些能力设置，请发送有效的 JSON 请求。
plugin-capability-model-migration = 把模型网关声明为可选能力需要更新版本的核心；模型网关迁移前，请继续使用 models 权限。
plugin-capability-plan-changed = 安装计划在预览后发生变化，请重新预览并确认连接。
plugin-capability-choices-unsupported = 这份核心无法应用所选设备或限定的 Bot 范围，未修改这些选择，也未开始安装。
plugin-capability-uninstall-selection = 所选提供方仍在使用中，或不在本次可卸载列表里，请重新打开卸载预览。
