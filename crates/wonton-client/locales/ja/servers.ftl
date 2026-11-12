# 複数のサーバー（crates/wonton-client/src/servers.rs、client/push_route.rs）。
servers-removed = このサーバーはこのスマートフォンから削除されています。

servers-push-from-removed = このメッセージを送ったサーバーはこのスマートフォンから削除されています。

servers-push-scope-closed = そのサーバーとの接続はすでに終了しています。

# $seconds は登録を待った秒数です。
servers-push-register-timeout = { $seconds }秒以内に通知を登録できませんでした。

# 登録済みサーバーのリスト（crates/wonton-client/src/servers.rs）。
# $path はサーバーリストファイルのパス、$reason は元のパースまたは入出力エラーです。
servers-list-corrupt = サーバーリストのファイル{ $path }が破損しています：{ $reason }

servers-add-current-unpaired = このサーバーを設定してから、別のサーバーを追加してください

servers-name-required = 名前を入力してください

# $path は読み書きまたは削除の対象のパス、$reason は元の入出力エラーです。
servers-storage-failed = { $path }を読み書きできません：{ $reason }

# シェルからコアの知らないサーバー一覧の操作が渡されたときに表示します。
servers-invalid-action = サーバー一覧の操作ではありません
