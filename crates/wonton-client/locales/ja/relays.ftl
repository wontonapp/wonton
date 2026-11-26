# リレーテーブル設定（ADR-0048）：読み取れない・保存できない理由です。
relays-not-paired = 自宅サーバーとまだペアリングしていません。

relays-not-connected = 自宅サーバーにまだ接続していません。

# リレーテーブルを保存中で、変更は保存が終わるまで待ちます。
relays-busy = リレーテーブルを保存中です。少し待ってから変更してください。
relays-not-loaded = リレーテーブルをまだ読み込めていません。先に再読み込みしてください。

# テーブルは WONTON_RELAYS 環境変数で固定されていて、ここでは変更できません。
relays-locked = リレーテーブルは環境変数で固定されていて、ここでは変更できません。

# $reason はプロキシURLパーサーの元のエラーです。
relays-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }

# $reason はHTTPクライアントの元のエラーです。
relays-http-client = HTTPクライアントを作成できません：{ $reason }

# $seconds はリクエストのタイムアウト秒数です。
relays-timeout = { $seconds }秒以内に応答がありませんでした

# $reason は上流の原文、または翻訳されたタイムアウト・プロトコルエラーの詳細です。
relays-request-failed = 自宅サーバーに接続できません：{ $reason }

# $reason はJSONパーサーの元のエラーです。
relays-invalid-response = 自宅サーバーが返したリレーテーブルの形式が正しくありません：{ $reason }

# $status は元のHTTPステータスコードです。
relays-server-status = 自宅サーバーがHTTP { $status } を返しました

# $message と $kind はサーバーの元のエラー説明とエラーコードです。
relays-server-error = { $message }（{ $kind }）

# $value は入力されたままのアドレスです。
relays-invalid-url = リレーのアドレスが無効です：{ $value }

# $value は入力されたままのアドレスです。
relays-needs-https = リレーのアドレスはhttpsでなければなりません：{ $value }

# $value は入力されたままのアドレスです。
relays-has-userinfo = リレーのアドレスにユーザー名やパスワードは入れられません。トークンは別に入力してください：{ $value }

# $value は正規化されたアドレスです。
relays-duplicate = そのリレーはすでにテーブルにあります：{ $value }

# $value は正規化されたアドレスです。
relays-missing = そのリレーはもうテーブルにありません：{ $value }

# $value は前に挿入する項目の正規化されたアドレスです。
relays-before-missing = 前に挿入するリレーはもうテーブルにありません：{ $value }

# $reason は接続情報パーサーの元のエラーです。
relays-invalid-ticket = 保存された接続情報が破損しています：{ $reason }
