## 家裡的伺服器自己更新的文字（ADR-0025）：App 的更新頁顯示的內容和詳細資訊裡的原因。
## 背景檢查和更新器寫文字用主人最近的語言；管理介面按請求的語言回答。
## 台灣用詞；對使用者說「家用伺服器」，不說「核心」；「升級」對使用者叫「更新」。
## 錯誤訊息結尾不加句號：幾條訊息會拼在一起顯示。

# 更新介面拒絕請求。
upgrade-not-installed = 家用伺服器不是正式安裝的，不能一鍵更新
upgrade-not-in-container = 家用伺服器不是在容器裡執行的，不能一鍵更新
upgrade-busy = 已經在更新了
upgrade-not-downloaded = 新版本還沒下載好
upgrade-channel-changed = 這個版本類型的最新版本變了，請重新確認
upgrade-pinned = 這次安裝固定在一個版本，不會跟著版本類型更新
upgrade-not-upgrading = 目前沒有在更新
upgrade-cancel-too-late = 更新已經開始變更，無法取消
upgrade-launch-updater-failed = 啟動更新器失敗：{ $error }

# 介面自己查不到要答的東西。
upgrade-container-missing = 找不到家用伺服器的容器 { $name }
upgrade-image-missing = 找不到正在使用的映像檔 { $image }；重新啟動家用伺服器後恢復

# 背景檢查（每 6 小時一次，打開更新頁也會查）。
upgrade-check-core-failed = 查家用伺服器的容器失敗：{ $error }
upgrade-latest-version-failed = 查不到 { $source } 的最新版本：{ $error }
upgrade-image-lookup-failed = 查映像檔失敗：{ $error }
upgrade-registry-digest-missing = 映像檔倉庫沒有回傳 { $source } 的摘要
upgrade-free-space-failed = 查 { $path } 的磁碟空間失敗：{ $error }
upgrade-pull-stalled = 取得 { $image } 連續 5 分鐘沒有進展
upgrade-auto-download-skipped = 磁碟剩不到 5 GB，沒有自動下載
upgrade-download-no-space = 磁碟不夠：下載大約要 { $needed } 位元組，只有 { $free } 位元組可用
upgrade-download-failed = 下載失敗：{ $error }
upgrade-image-missing-after-pull = 下載完了，本機還是沒有 { $reference }

# 相容性檢查（ADR-0085）查不成不攔升級：詳情如實說沒查成，升級照常可以發起。
upgrade-compat-records-unreadable = 讀不了已安裝應用程式的紀錄：{ $error }
upgrade-compat-materialize-failed = 無法為相容性檢查寫出已安裝應用程式的套件：{ $error }
upgrade-compat-run-failed = 在新映像檔 { $image } 裡跑不了相容性檢查：{ $error }
upgrade-compat-command-failed = 新映像檔 { $image } 裡的相容性檢查以結束代碼 { $code } 結束：{ $error }
upgrade-compat-output-unreadable = 讀不懂相容性檢查的輸出：{ $error }

# 更新器容器把自己做的事寫進狀態檔，用主人的語言。
upgrade-updater-crashed = 更新器反覆崩潰
upgrade-phase-unreadable = 無法讀取狀態檔裡的階段
upgrade-updater-gone = 更新器不見了，無法確定 { $phase } 這一步是否完成
upgrade-rollback-cause = { $message }。回復舊版本的原因：{ $cause }
upgrade-restore-also-failed = { $message }。之後啟動回原來的版本也失敗了：{ $error }
upgrade-state-unreadable = 無法讀取 { $path }，可能是較新的版本寫入的
upgrade-state-no-reason = 狀態檔裡沒有寫原因
upgrade-state-serialize = 無法序列化更新狀態
upgrade-no-backup-dir = 狀態檔裡沒有備份目錄
upgrade-restore-pin-hint = 要退回舊版本，在主機上再執行一次安裝指令，加上 WONTON_CORE_PIN={ $digest }

# 在主機上拉映像檔、跑容器。
upgrade-list-images-failed = 新映像檔 { $image } 列不出它要用的映像檔：{ $error }
upgrade-no-image = 本機沒有映像檔 { $image }
upgrade-inspect-image-failed = 查映像檔 { $image } 失敗：{ $error }
upgrade-pull-image-failed = 無法取得映像檔 { $image }：{ $error }
upgrade-pull-image-timed-out = 取得映像檔 { $image } 超過了時限
upgrade-run-in-image-failed = 在映像檔 { $image } 裡執行 { $cmd } 失敗：{ $error }
upgrade-window-exceeded = 從停止家用伺服器到新版本啟動，超過了 30 分鐘
upgrade-components-failed =
    { $target ->
        [new] 處理新版本不支援的元件失敗：{ $error }
       *[old] 處理舊版本不支援的元件失敗：{ $error }
    }
upgrade-up-failed = 服務沒有啟動：{ $error }
upgrade-core-image-wrong = 家用伺服器的容器跑的是 { $actual }（映像檔 { $actual_id }），不是 { $image }（映像檔 { $image_id }）
upgrade-core-container-missing = 沒有家用伺服器的容器
upgrade-containers-failed = 查系統服務的容器失敗：{ $error }
upgrade-observe-timed-out = 觀察階段超過了時限
upgrade-rollback-timed-out = 回復舊版本超過了 20 分鐘
upgrade-stop-service-failed = 停止服務 { $service } 失敗：{ $error }
upgrade-stop-core-failed = 停止家用伺服器失敗：{ $error }

# 備份和還原（哪一步出錯寫在狀態檔裡）。
upgrade-backup-failed = 備份失敗：{ $error }
upgrade-backup-incomplete = 備份 { $path } 沒有寫完，不能用來還原
upgrade-volume-size-failed = 查詢磁碟區 { $volume } 的大小失敗：{ $error }
upgrade-volume-size-unknown = 無法得知磁碟區 { $volume } 的大小：{ $error }
upgrade-disk-space-failed = 查磁碟空間失敗：{ $error }
upgrade-backup-no-space = 磁碟不夠：備份要 { $needed } 位元組，只有 { $free } 位元組可用，舊備份能騰出 { $old } 位元組
upgrade-backup-volume-failed = 備份磁碟區 { $volume } 失敗：{ $error }
upgrade-restore-failed = 還原備份失敗：{ $error }
upgrade-init-failed = 新版本的 init 失敗了：{ $error }

# 切換之後盯著各服務時發現的問題。
upgrade-service-container-gone = 服務 { $service } 的容器不見了
upgrade-service-container-replaced = 服務 { $service } 的容器被換掉了
upgrade-service-image-changed = 服務 { $service } 換了映像檔：{ $image }
upgrade-service-restarted = 服務 { $service } 重新啟動了
upgrade-service-status = 服務 { $service } 的狀態是 { $status }
upgrade-service-unhealthy = 服務 { $service } 的狀態變成異常
upgrade-one-shot-exit-code = 一次性服務 { $service } 的結束代碼是 { $code }

# 系統錯誤前面的短說明，如「讀取 /x: Permission denied」。
upgrade-env-missing-key = { $path } 裡缺 { $key }

# compose（核心和更新器裡）和一次性容器。
upgrade-compose-run-failed = 執行 { $path } 失敗：{ $error }
upgrade-compose-timed-out = compose 超過 { $seconds } 秒還沒結束
upgrade-compose-wait-failed = 等 compose 結束失敗：{ $error }
upgrade-compose-failed = compose 失敗（{ $status }）：{ $error }
upgrade-one-shot-timed-out = 容器 { $name } 超過 { $seconds } 秒還沒結束
upgrade-exit-code-unknown = 查不到容器 { $name } 的結束代碼
