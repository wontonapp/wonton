## 伺服器配對（ADR-0050）裡給 App 和聊天卡片看的文字（ADR-0043）。

peers-endpoint-detail = endpoint { $id }，上次錯誤：{ $error }
peers-endpoint = endpoint { $id }
peers-request-title = { $owner } 的伺服器想和你的伺服器連線
peers-request-server = 對方伺服器：{ $server }
peers-connect = 連線
peers-reject = 拒絕
peers-connected = 已連線
peers-error-code-expired = 配對碼過期了，請重新出一張碼
peers-error-code-invalid = 無法讀取配對碼：不是雲吞的伺服器配對碼，或者被改過
peers-error-peer-unreachable = 連不上對方的伺服器，稍後再試
peers-error-peer-rejected = 對方拒絕了配對
peers-error-already-paired = 兩台伺服器已經配對過
peers-error-self-pairing = 這是你自己伺服器的碼
peers-error-not-paired = 這台伺服器還沒有和你配對
peers-error-unauthenticated = 請先在雲吞裡配對，再從雲吞打開這一頁
peers-error-transport = 配對沒有完成，請稍後再試
peers-error-matrix-unavailable = 這台伺服器沒有接入 Matrix，配對用不了
peers-error-not-paired-remove = 沒有配對過這台伺服器
peers-error-code-revoked = 這張配對碼已經作廢，請對方重新發一張
peers-error-too-many-requests = 對方暫時收不了配對請求，稍後再試
peers-error-request-not-found = 這條請求已經不在了
