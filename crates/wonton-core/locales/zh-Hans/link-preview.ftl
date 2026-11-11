## 链接预览的错误（ADR-0065）。App 按类别显示自己的文字，这里的进“详情”。
## { $detail } 是原始值：地址、状态码、内容类型或底层错误。

link-preview-invalid-url = 只有 http、https 链接，端口是 80、443、8080 或 8443 的才有预览：{ $detail }
link-preview-blocked-address = 链接指向本机或内网地址，家里的服务器不去打开：{ $detail }
link-preview-dns-failed = 经代理解析这个域名失败，连不上对方网站：{ $detail }
link-preview-upstream-error = 对方网站连不上或出错了：{ $detail }
link-preview-timeout = 对方网站 { $detail } 内没有回应
link-preview-not-html = 链接不是网页：{ $detail }
link-preview-no-preview = 网页里没有标题、摘要和图片：{ $detail }
link-preview-too-many-requests = 这台设备取预览太频繁，过一分钟再试
link-preview-image-gone = 预览图片已经过期，重新取一次预览
