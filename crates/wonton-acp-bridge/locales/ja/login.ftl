## エージェントのログインフローの文（チャットコマンド、ログイン制御ソケット経由のターミナルログイン）。
## コマンド名は翻訳しません。

login-redact-reason = エージェントのログイン処理に引き渡し済み
login-answer-first = 先に /wonton login accept か /wonton login decline でログインリンクに回答してください。
login-code-limits = 認証コードは1行のテキストで、16 KiB 以下にしてください。
login-not-sent = ログインはすでに終了しています。このメッセージはエージェントに送信されていません。
login-no-forward = ログイン中は通常のチャットを転送しません。公式ページでログインを済ませるか、/wonton login cancel を送ってください。
login-no-direct = あなたとのダイレクトメッセージが見つかりません。/wonton login をそちらで送ってください。
login-in-direct = ログインはあなたとのダイレクトメッセージで行います。
login-cancelled = ログインを取り消しました。
login-no-pending = 回答待ちのログインリンクはありません。
login-finish-current = 現在の操作を終えてからログインを開始してください。
login-starting = エージェントのログインを開始しています。取り消すには /wonton login cancel を送ってください。
login-unsupported-method = このログイン方法にはまだ対応していません。
login-unknown-method = エージェントはこのログイン方法を提供していません。/wonton login で確認できます。
login-secret-delete = このメッセージにはログイン用の秘密が含まれています。ご自身で削除してください。
login-done = ログインしました。
login-no-methods = エージェントはログイン方法を提供していません。
login-methods-header = エージェントのログイン方法を選択してください：
login-method-line = { $name }：/wonton login { $id }
login-link =
    エージェントへのログイン：
    { $message }
    { $url }
    リンクを開いてログインし、/wonton login accept と返信してください。拒否する場合は /wonton login decline と返信します。

# エラー接頭辞の後ろに入る詳細（errors.ftl 参照）。
login-auth-failed = エージェントのログインに失敗しました（ACP { $code }）
login-auth-kind-unsupported = エージェントが提供するログイン方式に対応していません
login-timeout = ログインがタイムアウトしました。もう一度開始してください
login-socket-connect = ログイン制御ソケット { $path } に接続できません：{ $error }
login-invalid-frame = ログインソケットが無効なフレームを返しました
login-process-exited = エージェントのログイン処理が途中で終了しました
login-failed-detail = エージェントのログインに失敗しました：{ $detail }
login-exit-code = エージェントのログイン処理が終了コード { $code } で失敗しました
login-socket-closed = ログインソケットが切断されました：{ $error }
login-chat-closed = ログインのチャットが閉じました
