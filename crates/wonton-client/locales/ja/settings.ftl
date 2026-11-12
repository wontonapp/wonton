# 通知方式の設定（crates/wonton-client/src/push_method.rs）。
# $path は設定ファイルのパス、$reason は元の入出力エラーです。
push-method-write-failed = 通知方式の設定ファイル{ $path }に書き込めません：{ $reason }

# プッシュ通知（crates/wonton-client/src/push/{mod,registration,notification}.rs）：拒否されたエンドポイント、失敗した登録、メッセージを取得できなかった場合の詳細情報。
# 通知の本文とプレースホルダー（不明な送信者、招待、[ファイル]など）はまだここにはありません。
# $reason は元のエラーの詳細です。
push-matrix-pusher-failed = Matrixにpusherを登録できません：{ $reason }

# $reason はURLパーサーの元のエラーです。
push-endpoint-invalid-url = プッシュ通知のエンドポイントが有効なURLではありません：{ $reason }

push-endpoint-scheme = プッシュ通知のエンドポイントはhttpまたはhttpsのアドレスである必要があります

push-endpoint-p256dh = p256dhは65バイトの非圧縮P-256公開鍵（base64url）である必要があります

push-endpoint-auth = authは16バイト（base64url）である必要があります

# $reason は上流の原文、または翻訳されたタイムアウトの詳細です。
push-register-failed = 自宅サーバーにプッシュ通知を登録できません：{ $reason }

push-unregister-failed = 自宅サーバーのプッシュ通知の登録を解除できません：{ $reason }

# $reason はJSONパーサーの元のエラーです。
push-register-invalid-response = 自宅サーバーが返した登録結果の形式が正しくありません：{ $reason }

# $reason はローカルプロキシまたはHTTPクライアントの元のエラーです。
push-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }

push-http-client = HTTPクライアントを作成できません：{ $reason }

# $seconds はリクエストのタイムアウト秒数です。
push-timeout = { $seconds }秒以内に応答がありません

# $status はそのままのHTTPステータスコードです。
push-server-status = 自宅サーバーからHTTP { $status }が返されました

# $message と $kind はサーバーの元のエラーテキストとエラーコードです。
push-server-error = { $message }（{ $kind }）

# $reason はJSONパーサーの元のエラーです。
push-payload-invalid = プッシュ通知の内容の形式が正しくありません：{ $reason }

# $reason はIDパーサーの元のエラーです。
push-invalid-room-id = プッシュ通知内のルームIDが無効です：{ $reason }

push-invalid-event-id = プッシュ通知内のイベントIDが無効です：{ $reason }

# $seconds はメッセージの取得で待った秒数です。
push-fetch-timeout = { $seconds }秒以内にこのメッセージを取得できませんでした

# $reason はMatrix SDKの元のエラーです。
push-fetch-failed = このメッセージを取得できません：{ $reason }

push-core-closed = クライアントコアが終了しました

push-not-paired = まだ自宅サーバーに接続したことがありません

push-home-unreachable = 自宅サーバーに接続できません：{ $reason }

# $seconds はログインで待った秒数です。
push-session-timeout = { $seconds }秒以内にMatrixにログインできませんでした

push-room-key-missing = このメッセージのルームキーがまだ届いていません
