chat-sign-in-no-connection-info = 自宅サーバーの接続情報がまだありません

chat-sign-in-no-saved-session = ログイン後にログイン状態を取得できません

# ログイントークンの取得リクエスト。$reason は上流の原文、または翻訳されたタイムアウトの詳細です。
# $reason はプロキシURLパーサーの元のエラーです。
chat-login-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }

chat-login-request-failed = 自宅サーバーからMatrixログイントークンを取得できません：{ $reason }

# $message と $kind はサーバーの元のエラーメッセージとエラーコードです。
chat-login-server-error = { $message }（{ $kind }）

# 復旧関連。
chat-recovery-not-connected = 自宅サーバーにまだ接続していません

chat-recovery-reset-no-response = 自宅サーバーが応答しません。バックアップは変更していません。後でもう一度お試しください。

chat-recovery-empty-key = 復旧キーが空です

# 回答や時間枠の停止が失敗したときに、確認カードの下の問題の詳細に表示される文章です。
# $message は自宅サーバーの元のエラーメッセージ、または翻訳された詳細です。
chat-confirmation-answer-failed = 回答できませんでした：{ $message }

# $error はHTTPクライアントの元のエラーです。
chat-confirmation-answer-lost = 自宅サーバーからの応答がなく、回答が届いたかはっきりしません。もう一度試せます（{ $error }）

# $message は自宅サーバーの元のエラーメッセージ、または翻訳された詳細です。
chat-confirmation-stop-failed = 停止できませんでした：{ $message }

# $error はJSONパーサーの元のエラーです。
chat-confirmation-response-unreadable = 自宅サーバーの応答を読み取れません（{ $error }）

# $status は元のHTTPステータスコードです。
chat-confirmation-core-status = 自宅サーバーがHTTP { $status }を返しました

chat-confirmations-list-failed = 確認リストを読み込めませんでした：{ $message }

# $reason はMatrix IDパーサーの元のエラーです。
chat-contacts-invalid-user-id = ユーザーIDが無効です：{ $reason }

chat-layout-invalid-bot-id = Matrix IDの形式が正しくありません（@name:server の形式）。

chat-layout-change-failed = チャットの表示方法を変更できませんでした：{ $reason }

# Matrixがこのデバイスを取り消し、アカウントが失敗扱いになったときに表示します。
chat-session-device-revoked = このデバイスはMatrixで取り消されました。もう一度自宅サーバーとペアリングしてください。

# ダイレクトチャットの開始時に、@名前:サーバーの形でないIDが渡されたときに表示します。
chat-invalid-user-id = ユーザーIDの形式が正しくありません。@名前:サーバーのように入力してください

# $file_type は選択したファイルの元のMIMEタイプです。
chat-not-image-type = 画像ファイルではありません：{ $file_type }

# 自宅サーバーに接続する前に確認へ回答したり停止したりしたときに表示します。
chat-action-not-connected = 自宅サーバーにまだ接続していません

# $error はルームID解析の元のエラーです。
chat-invalid-room-id = ルームIDが正しくありません：{ $error }

# $room は元のルームIDです。
chat-room-not-subscribed = 先にルーム{ $room }のタイムラインを購読してください

# デフォルトのエージェントを選ぶ前に問い合わせたときに表示します。
chat-no-default-agent = デフォルトのエージェントがまだありません。設定で選んでください

chat-op-display-name-failed = 表示名を同期できませんでした：{ $error }

chat-op-resend-failed = メッセージを再送できませんでした：{ $error }

chat-op-send-image-failed = 画像を送信できませんでした：{ $error }

chat-op-forward-failed = メッセージを転送できませんでした：{ $error }

chat-op-redact-failed = メッセージを取り消せませんでした：{ $error }

chat-avatar-cache-not-file = アバターのキャッシュのパスがファイルではありません

chat-avatar-cache-missing-mxc = アバターのキャッシュにMXCアドレスがありません

chat-avatar-cache-path-not-utf8 = アバターのキャッシュのパスが有効なUTF-8ではありません

# チャットのローカルストレージ（crates/wonton-client/src/chat/storage.rs）。
# $path はファイルのパス、$reason は元のパースまたは入出力エラーです。
chat-storage-session-corrupt = チャットのログイン状態のファイル{ $path }が破損しています：{ $reason }

chat-storage-io-failed = { $path }を読み書きできません：{ $reason }

# チャットの接続問題（crates/wonton-client/src/chat/view.rs）：CoreError::TransportUnavailableの理由と失敗した操作のメッセージ。
chat-not-connected = 自宅サーバーに接続してから、もう一度試してください

chat-draft-save-failed = 下書きの保存に失敗しました：{ $error }

chat-draft-not-signed-in = まだ Matrix にログインしていません

# 確認カードの下の、許可期間が終わったあとの行です。$count は期間を開いた1回を除いて許可した回数で、0 は不明またはなしです。
chat-window-ended =
    { $reason ->
        [expired]
            { $count ->
                [0] 許可期間が終了しました。
               *[other] 許可期間が終了しました（{ $count }回許可）。
            }
        [stopped]
            { $count ->
                [0] 許可期間を停止しました。
               *[other] 許可期間を停止しました（{ $count }回許可）。
            }
        [restarted] サーバーが再起動したため、許可期間は終了しました。
       *[other]
            { $count ->
                [0] 許可期間が終了しました。
               *[other] 許可期間が終了しました（{ $count }回許可）。
            }
    }
