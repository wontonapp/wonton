## 自宅サーバー自身のアップデートに関する文（ADR-0025）：Appの更新ページに表示される内容と
## 詳細の理由。バックグラウンドのチェックとアップデーターはオーナーの言語で書き、管理APIは
## リクエストの言語で答える。エラー文の末尾には句点を付けない（複数の文をつなげて表示するため）。

# 更新APIがリクエストを拒否したとき。
upgrade-not-installed = この自宅サーバーは標準的な方法でインストールされていないため、ワンクリックアップデートは使えません
upgrade-not-in-container = 自宅サーバーがコンテナで動いていないため、ワンクリックアップデートは使えません
upgrade-busy = アップデートはすでに実行中です
upgrade-not-downloaded = 新しいバージョンはまだダウンロードできていません
upgrade-channel-changed = アップデートチャンネルのバージョンが変わりました。もう一度確認してください
upgrade-pinned = このインストールはバージョン固定のため、アップデートチャンネルに従いません
upgrade-not-upgrading = 実行中のアップデートはありません
upgrade-cancel-too-late = すでに変更を始めているため、キャンセルできません
upgrade-launch-updater-failed = アップデーターを起動できませんでした：{ $error }

# API 自身が確認できなかったとき。
upgrade-container-missing = 自宅サーバーのコンテナ{ $name }が見つかりません
upgrade-image-missing = 実行中のイメージ{ $image }が見つかりません

# バックグラウンドのチェック（6時間ごと。更新ページを開いたときも行う）。
upgrade-check-core-failed = 自宅サーバーのコンテナを確認できませんでした：{ $error }
upgrade-latest-version-failed = { $source }の最新バージョンを確認できませんでした：{ $error }
upgrade-image-lookup-failed = イメージを確認できませんでした：{ $error }
upgrade-registry-digest-missing = レジストリが{ $source }のダイジェストを返しませんでした
upgrade-free-space-failed = { $path }の空き容量を確認できませんでした：{ $error }
upgrade-pull-stalled = { $image }の取得が5分間進みませんでした
upgrade-auto-download-skipped = 空き容量が5GB未満のため、自動ダウンロードしませんでした
upgrade-download-no-space = 空き容量が足りません：ダウンロードには約{ $needed }バイト必要ですが、利用できるのは{ $free }バイトです
upgrade-download-failed = ダウンロードできませんでした：{ $error }
upgrade-image-missing-after-pull = ダウンロード後も{ $reference }がこのホストに見つかりません

# アップデーターは自分の作業を状態ファイルに書き込みます（オーナーの言語で）。
upgrade-updater-crashed = アップデーターが繰り返しクラッシュしています
upgrade-phase-unreadable = 状態ファイルのフェーズを読み取れません
upgrade-updater-gone = アップデーターが見つからず、{ $phase }まで進んだか分かりません
upgrade-rollback-cause = { $message }。ロールバックの原因となった問題：{ $cause }
upgrade-restore-also-failed = { $message }。その後、旧バージョンの起動も失敗しました：{ $error }
upgrade-state-unreadable = { $path }を読み取れません。新しいバージョンが書いた可能性があります
upgrade-state-no-reason = 状態ファイルに理由が書かれていません
upgrade-state-serialize = アップデートの状態をシリアライズできません
upgrade-no-backup-dir = 状態ファイルにバックアップディレクトリがありません
upgrade-restore-pin-hint = 旧バージョンに戻すには、ホストでインストールコマンドにWONTON_CORE_PIN={ $digest }を付けてもう一度実行します

# ホスト上でのイメージ取得とコンテナ実行。
upgrade-list-images-failed = 新しいイメージ{ $image }が必要なイメージを一覧できませんでした：{ $error }
upgrade-no-image = イメージ{ $image }がこのホストにありません
upgrade-inspect-image-failed = イメージ{ $image }を確認できませんでした：{ $error }
upgrade-pull-image-failed = イメージ{ $image }を取得できませんでした：{ $error }
upgrade-pull-image-timed-out = イメージ{ $image }の取得が時間切れになりました
upgrade-run-in-image-failed = イメージ{ $image }内で{ $cmd }を実行できませんでした：{ $error }
upgrade-window-exceeded = 自宅サーバーを止めてから新しいバージョンが起動するまで、30分を超えました
upgrade-components-failed =
    { $target ->
        [new] 新しいバージョンが対応していないコンポーネントを処理できませんでした：{ $error }
       *[old] 旧バージョンが対応していないコンポーネントを処理できませんでした：{ $error }
    }
upgrade-up-failed = サービスが起動しませんでした：{ $error }
upgrade-core-image-wrong = 自宅サーバーのコンテナは{ $actual }を使っており、{ $image }ではありません
upgrade-core-container-missing = 自宅サーバーのコンテナが見つかりません
upgrade-containers-failed = システムサービスのコンテナを確認できませんでした：{ $error }
upgrade-observe-timed-out = 新しいバージョンの監視が時間切れになりました
upgrade-rollback-timed-out = ロールバックが20分を超えました
upgrade-stop-service-failed = サービス{ $service }を停止できませんでした：{ $error }
upgrade-stop-core-failed = 自宅サーバーを停止できませんでした：{ $error }

# バックアップと復元（どの段階で失敗したかは状態ファイルに書かれています）。
upgrade-backup-failed = バックアップできませんでした：{ $error }
upgrade-backup-incomplete = バックアップ{ $path }は書き込みが完了していないため、復元に使えません
upgrade-volume-size-failed = ボリューム{ $volume }のサイズを確認できませんでした：{ $error }
upgrade-volume-size-unknown = ボリューム{ $volume }のサイズが分かりません：{ $error }
upgrade-disk-space-failed = 空きディスク容量を確認できませんでした：{ $error }
upgrade-backup-no-space = 空き容量が足りません：バックアップには{ $needed }バイト必要で、利用できるのは{ $free }バイト、古いバックアップで{ $old }バイト解放できます
upgrade-backup-volume-failed = ボリューム{ $volume }をバックアップできませんでした：{ $error }
upgrade-restore-failed = バックアップを復元できませんでした：{ $error }
upgrade-init-failed = 新しいバージョンのinitが失敗しました：{ $error }

# 切り替え後に各サービスを監視して見つかった問題。
upgrade-service-container-gone = サービス{ $service }のコンテナが見つかりません
upgrade-service-container-replaced = サービス{ $service }のコンテナが入れ替わりました
upgrade-service-image-changed = サービス{ $service }のイメージが変わりました：{ $image }
upgrade-service-restarted = サービス{ $service }が再起動しました
upgrade-service-status = サービス{ $service }の状態は{ $status }です
upgrade-service-unhealthy = サービス{ $service }が正常でなくなりました
upgrade-one-shot-exit-code = 1回だけ実行するサービス{ $service }が終了コード{ $code }で終了しました

# システムのエラーの前に付く短い説明（「{ $path }を読み取れません: Permission denied」のような形）。
upgrade-env-missing-key = { $path }に{ $key }がありません

# compose（コアとアップデーターの両方）と 1 回きりのコンテナ。
upgrade-compose-run-failed = { $path }を実行できませんでした：{ $error }
upgrade-compose-timed-out = composeが{ $seconds }秒以内に終了しませんでした
upgrade-compose-wait-failed = composeの終了を待てませんでした：{ $error }
upgrade-compose-failed = composeが失敗しました（{ $status }）：{ $error }
upgrade-one-shot-timed-out = コンテナ{ $name }が{ $seconds }秒以内に終了しませんでした
upgrade-exit-code-unknown = コンテナ{ $name }の終了コードが分かりません
