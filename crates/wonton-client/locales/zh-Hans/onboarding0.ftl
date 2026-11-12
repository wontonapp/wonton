onboarding-steps-need-not-listed = 这个需求不在推荐列表里

onboarding-steps-model-not-chosen = 还没有选模型

onboarding-steps-cannot-skip = 这一步不能跳过

onboarding-model-testing = 已经在测试

onboarding-model-custom-unsupported = 模型网关不支持自定义服务商

onboarding-model-base-url-invalid = Base URL 要是 https://… 这样的地址

onboarding-review-item-missing = 最后一页上没有这一项

# 引导的本机记录（crates/wonton-client/src/onboarding/storage.rs）。
# $path 是文件的路径，$reason 是原始的读写错误。
onboarding-storage-io-failed = 读写不了 { $path }：{ $reason }

# 资料这一步：名字或邮箱为什么存不了。$max 是名字最多几个字。
onboarding-profile-name-empty = 名字不能为空

onboarding-profile-name-control = 名字里不能有换行和控制字符

onboarding-profile-email-cleared = 邮箱填过以后只能改，不能清空：要邮箱的应用重建时会用到它
