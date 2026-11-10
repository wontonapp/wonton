# 転送や再生を始める前に接続がないとわかったときに表示されます。
files-api-not-connected = まだ自宅サーバーに接続していません

# $reason はプロキシアドレスのパーサーの元のエラーです。
files-api-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }

# $reason はHTTPクライアントの元のエラーです。
files-api-http-client = HTTPクライアントを作成できません：{ $reason }

# $reason はJSONパーサーの元のエラーです。
files-api-invalid-response = 自宅サーバーの応答を読み取れません：{ $reason }

# 自宅サーバーがこのデバイスを認証しなくなったときに表示されます。
files-api-unauthenticated = 自宅サーバーがこのデバイスを認識しなくなりました

# $code はトンネルの元のエラーコードです。
files-api-tunnel-unavailable = トンネルを利用できません（{ $code }）

# $status は元のHTTPステータスコードです。
files-api-status = 自宅サーバーからHTTP { $status }が返されました

# 元のファイルが選択時と変わっていたときに表示されます。
files-upload-source-changed = 元のファイルが変更されています。選び直してください。

# 進行状況の照会（HEAD）への自宅サーバーの応答にUpload-Offsetヘッダーがないときに表示されます。
files-upload-head-no-offset = 自宅サーバーの進行状況の応答にUpload-Offsetがありません

# シェルがこの種類のソースを開けないときに表示されます。
files-upload-no-opener = シェルがFileOpenerを設定していないため、このソースを開けません

# ソースがシークできる通常のファイルではないときに表示されます。
files-upload-source-not-file = このソースはシークできる通常のファイルではないため、中断した位置から再開できません

# $reason は元のファイルを開くタスクの異常終了の情報です。
files-upload-open-task-failed = 元のファイルを開くタスクが異常終了しました：{ $reason }

# $reason は元のioエラーです。
files-upload-source-denied = システムが元のファイルの読み取りを許可していません：{ $reason }

# $reason は元のioエラーです。
files-upload-source-missing = 元のファイルが見つかりません：{ $reason }

# $reason は元のioエラーです。
files-upload-source-read = 元のファイルを読み取れません：{ $reason }

# アップロード作成への自宅サーバーの応答に有効なLocationヘッダーがないときに表示されます。
files-upload-create-no-location = 自宅サーバーのアップロード作成の応答に有効なLocationがありません

# 進行状況の照会（HEAD）への自宅サーバーの応答にUpload-Lengthヘッダーがないときに表示されます。
files-upload-head-no-length = 自宅サーバーの進行状況の応答にUpload-Lengthがありません

# 停滞の制限時間を過ぎてもデータが動いていないときに表示されます。
files-upload-stalled = アップロードが進まず、トンネルが停止している可能性があります

# 最後のPATCHに自宅サーバーが応答しないときに表示されます。
files-upload-no-response = 自宅サーバーから応答がありません

# 元のファイルが記録より短くなっていたときに表示されます。
files-upload-source-shrank = 元のファイルが短くなりました

# $reason は元のioエラーです。
files-download-write = ダウンロードしたファイルを書き込めません：{ $reason }

# ダウンロードの要求に自宅サーバーが応答しないときに表示されます。
files-download-no-response = 自宅サーバーから応答がありません

# 部分ファイルが自宅サーバーのファイルより長いときに表示されます。
files-download-shrunk = ファイルが短くなったため、最初からダウンロードし直します

# 再開の範囲がダウンロード済みの内容と一致しないときに表示されます。
files-download-range-mismatch = 自宅サーバーから返された範囲が一致しないため、最初からダウンロードし直します

# 接続が切断され、すべてのデータを受信できなかったときに表示されます。
files-download-incomplete = 接続が切断され、最後まで受信できませんでした

# 停滞の制限時間を過ぎてもデータが届かないときに表示されます。
files-download-stalled = データを受信できず、トンネルが停止している可能性があります

# 接続の切断によって再生が失敗したときに表示されます。
files-playback-transport = 自宅サーバーに接続できません。

# この端末でこの形式を再生できないときに表示されます。
files-playback-unsupported = この端末ではこの形式を再生できません。ダウンロードしてから他のアプリで開けます。

# $detail は自宅サーバーまたは接続の元の理由です。
files-playback-unavailable = 自宅サーバーがこのファイルを提供できません：{ $detail }

# $detail は自宅サーバーまたは接続の元の理由です。
files-playback-error = 自宅サーバーでエラーが発生しました：{ $detail }。

# 再生の確認がタイムアウトしたときに表示されます。
files-playback-no-response = 自宅サーバーから応答がありません

# $reason は元のioエラーです。
files-stream-local-address = 再生用のローカルアドレスを開けません：{ $reason }

# トンネル未接続時の再生失敗の診断詳細です。
files-stream-not-connected = トンネルに接続していません

# $code はトンネルの元のエラーコードです。
files-stream-tunnel-unavailable = トンネルを利用できません（{ $code }）

# ナビゲーションバー ViewModel のエラー。CoreError の理由、またはナビゲーションバーの問題表示として出ます。
# 引数はありません。
tabs-view-always-pinned = チャットとアプリはすでにナビゲーションバーに固定されています。

tabs-view-plugin-list-not-ready = アプリの一覧をまだ読み込んでいないため、今はアプリのページを操作できません。

tabs-view-unknown-plugin = このアプリが見つかりません。

# $error はローカル保存の元のエラー。
tabs-storage-problem = マイアプリを保存できません。次回の起動では古い状態に戻ることがあります: { $error }

tabs-view-plugin-no-page = このアプリには開けるウェブページがありません。

tabs-page-not-placed = このページはパネルにありません。

# ナビゲーションバーの設定ファイル（tabs.json）のエラー。CoreError の理由として出ます。
# $path は設定ファイルのパス、$reason は元のパースまたは入出力エラーです。
tabs-config-read-failed = マイアプリの設定ファイル { $path } を読み込めません：{ $reason }

tabs-config-corrupt = マイアプリの設定ファイル { $path } が破損しています：{ $reason }

# $count はナビゲーションバーに置ける項目の上限（MAX_ITEMS）です。
tabs-config-too-many-items = ナビゲーションバーには最大 { $count } 個までしか項目を追加できません。

tabs-config-duplicate-page = マイアプリに重複したページがあります。

tabs-config-plugin-page-incomplete = アプリのページにタイトルとアイコンがありません。

tabs-config-missing-fixed = ナビゲーションバーにチャットがありません。

# $path は設定ファイルのパス、$reason は元の書き込みエラーです。
tabs-config-write-failed = マイアプリの設定ファイル { $path } に書き込めません：{ $reason }

scope-task-crashed = バックグラウンドのタスクが異常終了しました：{ $error }

scope-switched-server = 別のサーバーに切り替えました

## クライアントコアの残りの部分（ADR-0043）：C1とC2が対象外だったモジュールの「詳細」で、既定では折りたたまれます。同じファイルのログは英語で書かれ、ここには含まれません。
## プッシュ通知の本文とプレースホルダー（不明な送信者、招待、[ファイル]など）はまだ未定のため、今回の範囲外です。
# ファイル転送（crates/wonton-client/src/files/mod.rs）：拒否された操作、失敗した操作、ローカルファイルの削除の詳細情報。
files-invalid-action = ファイルの操作ではありません

files-name-required = ファイル名を入力してください

files-download-already-running = このファイルはすでにダウンロード中です

files-transfer-not-found = この転送が見つかりません

# リンク再送の待機状態にない転送に対してリンクの再送を要求した場合に表示します。
files-transfer-server-changed = 自宅サーバーが変わったため、この転送は前のサーバーのものです

files-dismiss-transfer-running = この転送は実行中です。先にキャンセルしてください

# アップロードを削除してローカルファイルも削除しようとした場合に表示します。元のファイルはシステムから提供されたもので、Wontonでは削除できません。
files-dismiss-upload-source-protected = アップロードの元ファイルはWontonでは管理していません

# $reason は元のioエラーです。
files-delete-local-failed = スマートフォン上のファイルを削除できません：{ $reason }

# バックグラウンドのファイル操作の失敗結果。$id はそのままのファイルID、$message は元の失敗の詳細です。
files-delete-failed = ファイル{ $id }を削除できません：{ $message }

files-send-failed = ファイル{ $id }をデバイスに送れません：{ $message }

files-grant-failed = ファイル{ $id }をチャットに許可できません：{ $message }

files-play-failed = ファイル{ $id }を再生できません：{ $message }

# $path は転送リストのパス、$reason は元の入出力エラーです。
files-store-write-failed = 転送リスト{ $path }に書き込めません：{ $reason }

# セッションリスト（crates/wonton-client/src/session_store.rs）。
sessions-title-required = セッションのタイトルを入力してください

# $id はそのままのセッションIDです。
sessions-not-found = そのセッションはありません：{ $id }

# シェルからコアの知らないセッション操作が渡されたときに表示します。
sessions-invalid-action = セッション操作ではありません

# アプリページのブリッジ（crates/wonton-client/src/tabs/bridge/）。
tabs-bridge-page-not-open = このアプリのページは開かれていません

tabs-bridge-invalid-environment = ページのインセットとセーフエリアは有限の非負の数である必要があります

tabs-bridge-public-page = 公開ウェブページには Wonton のブリッジがありません。

tabs-page-cannot-reload = このページは再読み込みできません。

tabs-bar-not-pinned = このページはナビゲーションバーに固定されていません。

tabs-bar-already-pinned = このページはすでにマイアプリにあります。

tabs-bar-replace-required = ナビゲーションバーがいっぱいです。置き換えるページを指定してください。

tabs-bar-replacing-missing = 置き換えるページがナビゲーションバーにありません。

tabs-bar-fixed = チャットとアプリは置き換えられません。

tabs-bar-not-full = ナビゲーションバーがいっぱいではないので、置き換えるページは指定できません。

tabs-bar-remove-fixed = チャットとアプリは外せません。

tabs-bar-page-missing = このページはマイアプリにありません。

tabs-bar-index-out-of-range = その位置はマイアプリの範囲外です。

tabs-open-only-plugins = 開き方を設定できるのはアプリのページだけです。

tabs-fullscreen-only-plugins = 全画面にできるのはアプリのページだけです。

tabs-fullscreen-none = 全画面のページはありません。

tabs-fullscreen-not-current = そのページは全画面のページではありません。

tabs-widget-not-signed-in = 先にチャットにサインインしてから、ルームから開いてください。

tabs-widget-not-declared = このアプリはルームの中では開けません。

tabs-widget-not-pinnable = ルームの中で開いたアプリはピン留めできません。

tabs-view-not-a-page = ここで開けるのはアプリのページだけです。
tabs-view-no-page-open = 開いているアプリのページはありません。

# 出错页默认收起的"详情"（ADR-0032）：核心按 App 语言写，错误码收在里面。
tabs-load-details-transport = 自宅サーバに接続できるようになると、ページは自動で再読み込みします。

tabs-load-details-not-ready = ページの読み込み時にアプリの準備ができていません（HTTP {$code}）。

tabs-load-details-upstream = アプリのページがエラーを返しました（HTTP {$code}）。
# Owner-only generated plugin keys; input and collected secrets are never returned.
plugins-owner-secret-not-listed = 開いている一覧にこのキーはありません：{ $name }
