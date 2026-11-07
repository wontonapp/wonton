## 服务器配对（ADR-0050）里给 App 和聊天卡片看的文字（ADR-0043）。

peers-endpoint-detail = endpoint { $id }，上次错误：{ $error }
peers-endpoint = endpoint { $id }
peers-request-title = { $owner } 的服务器想和你的服务器连接
peers-request-server = 对方服务器：{ $server }
peers-connect = 连接
peers-reject = 拒绝
peers-connected = 已连接
peers-error-code-expired = 配对码过期了，请重新出一张码
peers-error-code-invalid = 配对码读不懂：不是云吞的服务器配对码，或者被改过
peers-error-peer-unreachable = 连不上对方的服务器，稍后再试
peers-error-peer-rejected = 对方拒绝了配对
peers-error-already-paired = 两台服务器已经配对过
peers-error-self-pairing = 这是你自己服务器的码
peers-error-not-paired = 这台服务器还没有和你配对
peers-error-unauthenticated = 请先在 App 里配对，再从 App 里打开这一页
peers-error-transport = 配对没有完成，请稍后再试
peers-error-matrix-unavailable = 这台服务器没有接入 Matrix，配对用不了
peers-error-not-paired-remove = 没有配对过这台服务器
peers-error-code-revoked = 这张配对码已经作废，请对方重新发一张
peers-error-too-many-requests = 对方暂时收不了配对请求，稍后再试
peers-error-request-not-found = 这条请求已经不在了
