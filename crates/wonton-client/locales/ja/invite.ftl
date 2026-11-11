# デバイスの追加時に、ペアリングコードをまだ取得できない理由です。
invite-not-paired = 自宅サーバーとまだペアリングしていません。

# $reason は接続情報パーサーの元のエラーです。
invite-invalid-ticket = 保存された接続情報が破損しています：{ $reason }

invite-not-connected = 自宅サーバーにまだ接続していません。

# $reason はプロキシURLパーサーの元のエラーです。
invite-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }

# $reason はHTTPクライアントの元のエラーです。
invite-http-client = HTTPクライアントを作成できません：{ $reason }

# $seconds はリクエストのタイムアウト秒数です。
invite-timeout = { $seconds }秒以内に応答がありませんでした

# $reason は上流の原文、または翻訳されたタイムアウト・プロトコルエラーの詳細です。
invite-request-failed = 自宅サーバーからペアリングコードを取得できません：{ $reason }

# $reason はJSONパーサーの元のエラーです。
invite-invalid-response = 自宅サーバーから返されたペアリングコードの形式が正しくありません：{ $reason }

# $status は元のHTTPステータスコードです。
invite-server-status = 自宅サーバーからHTTP { $status }が返されました

# $message と $kind はサーバーのエラーメッセージとエラーコードです。
invite-server-error = { $message }（{ $kind }）
