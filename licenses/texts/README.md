# 许可全文兜底

Android 依赖的许可来自它 POM 里的一行 SPDX 声明，本身不带全文；xcode 的 swift 包同理。这里放这些许可的标准全文，
生成清单时正文为空就取这里的。文件来自 SPDX 许可清单 v3.27.0（<https://github.com/spdx/license-list-data>），
`Apache-2.0 WITH LLVM-exception` 取 `Apache-2.0`。新增 APPROVED 里的 id 时一起补文件。

Rust crate 自己的许可全文来自 crate 源码，比这里更准确（带署名），不会用到这里。
