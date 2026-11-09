## 連結預覽的錯誤（ADR-0065）。App 按類別顯示自己的文字，這裡的進「詳細資訊」。
## { $detail } 是原始值：位址、狀態碼、內容類型或底層錯誤。

link-preview-invalid-url = 只有 http、https 連結，連接埠是 80、443、8080 或 8443 的才有預覽：{ $detail }
link-preview-blocked-address = 連結指向本機或內部網路位址，家用伺服器不去開啟：{ $detail }
link-preview-dns-failed = 經代理解析這個網域名稱失敗，連不上對方網站：{ $detail }
link-preview-upstream-error = 對方網站連不上或出錯了：{ $detail }
link-preview-timeout = 對方網站 { $detail } 內沒有回應
link-preview-not-html = 連結不是網頁：{ $detail }
link-preview-no-preview = 網頁裡沒有標題、摘要和圖片：{ $detail }
link-preview-too-many-requests = 這台裝置取得預覽太頻繁，過一分鐘再試
link-preview-image-gone = 預覽圖片已經過期，重新取得一次預覽
