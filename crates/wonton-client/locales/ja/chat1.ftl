## chat-a フラグメント：crates/wonton-client/src/chat/{login,sign_in,recovery}.rs。
## チャットの問題・エラーの折りたたまれた「詳細」です。CoreError の理由と ChatProblem の message です。
# サインイン関連。$reason は Matrix SDK の元のエラーの詳細です。
chat-sign-in-restore-failed = Matrixセッションを復元できません：{ $reason }

chat-sign-in-login-failed = Matrixにログインできません：{ $reason }

# $reason はHTTPクライアントの元のエラーです。
chat-login-http-client = HTTPクライアントを作成できません：{ $reason }

# $seconds はリクエストのタイムアウト秒数です。
chat-login-timeout = { $seconds }秒以内に応答がありませんでした

# $reason はHTTPレスポンスの読み取りの元のエラーです。
chat-login-response-failed = 自宅サーバーからの応答を読み取れません：{ $reason }

# $reason はJSONパーサーの元のエラーです。
chat-login-invalid-token = 自宅サーバーから返されたログイントークンの形式が正しくありません：{ $reason }

# $login_type は自宅サーバーが返したログイン方式、$expected は要求されたログイン方式です。
chat-login-wrong-type = 自宅サーバーが返したログイン方式{ $login_type }は{ $expected }ではありません

# $status は元のHTTPステータスコードです。
chat-login-server-status = 自宅サーバーからHTTP { $status }が返されました

# $reason はストレージの元のエラーの詳細です。
chat-recovery-key-not-saved = 復旧キーをこのデバイスに保存できませんでした。今書き写してください：{ $reason }

# $error はHTTPクライアントの元のエラーです。
chat-confirmation-stop-lost = 自宅サーバーからの応答がありませんでした。もう一度試せます（{ $error }）

# 個人チャットを始めるとき、ユーザー名が空の場合に表示します。
chat-contacts-empty-username = ユーザー名を入力してください。

# Siri、ショートカット、チャット画面でメッセージを送るとき、本文が空の場合に表示します。
chat-empty-message = メッセージを入力してください。

# Siriやショートカットから渡されたボットのIDを解析できない場合に表示します。
chat-ask-invalid-bot-id = デフォルトのエージェントのIDの形式が正しくありません。

# 既存のダイレクトチャットのルームIDを解析できない場合に表示します。
chat-ask-invalid-room-id = ダイレクトチャットのルームIDが無効です。

# シェルからコアの知らないチャット操作が渡されたときに表示します。
chat-invalid-action = チャット操作ではありません

# シェルからコアの知らない回答が渡されたときに表示します。
chat-unknown-answer = このバージョンのWontonでは、この回答を認識できません

# Matrixにサインインする前にチャット操作が届いたときに表示します。
chat-not-signed-in = Matrixにまだログインしていません

chat-send-no-response = Matrixサーバーが応答しなくなりました

# 通知専用のルームに送ろうとしたときに表示します。
chat-read-only-room = このルームは通知専用のため、メッセージを送信できません

# $error はイベント ID パーサーの元のエラー。
chat-invalid-event-id = メッセージのイベントIDが正しくありません：{ $error }

# 転送・取り消しするメッセージのメニューにその項目がない（取り消し済み、未送信、権限がないなど）ときに表示。
chat-message-action-unavailable = このメッセージにはその操作ができません

# 転送先のルームが存在しないか、まだ参加していないときに表示。
chat-forward-target-not-joined = 転送先のルームにまだ参加していません

# バックグラウンドのチャット操作が失敗したときの問題表示です。$error はMatrix SDKまたは自宅サーバーの元のエラーです。
chat-op-ask-agent-failed = デフォルトのエージェントに質問できませんでした：{ $error }

chat-op-join-failed = ルームに参加できませんでした：{ $error }

chat-op-leave-failed = ルームから退出できませんでした：{ $error }

chat-op-cancel-send-failed = 送信をキャンセルできませんでした：{ $error }

chat-op-paginate-failed = 過去のメッセージを読み込めませんでした：{ $error }

chat-op-mark-read-failed = ルームを既読にできませんでした：{ $error }

chat-op-direct-chat-failed = ダイレクトチャットを開始できませんでした：{ $error }

chat-op-send-text-failed = メッセージを送信できませんでした：{ $error }

chat-avatar-cache-hash-collision = アバターのキャッシュでハッシュが衝突しています

chat-draft-clear-failed = 送信済みの下書きを消すのに失敗しました：{ $error }

chat-draft-read-interrupted = 下書きの読み込みが中断されました
