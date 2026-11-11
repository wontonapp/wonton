## 折りたたまれた技術的な詳細です。原因、エラーコード、上流の原文は変更しません。
## 長さの制限はありません。引数は診断データで、翻訳キーではありません。

# CoreErrorの説明です。$reason は元のエラーの詳細です。
core-error-storage = ローカルストレージを読み書きできません：{ $reason }
core-error-not-ready = まだ準備ができていません：{ $reason }
core-error-transport = 自宅サーバーに接続できません：{ $reason }
core-error-unauthorized = 自宅サーバーがリクエストを拒否しました：{ $reason }
core-error-upstream = 自宅サーバーでエラーが発生しました：{ $reason }
core-error-server-too-old = 自宅サーバーが古いため、この操作はまだできません。サーバーを更新してからもう一度お試しください。

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

# 他のサーバー（ADR-0050）。既定で折りたたまれる「詳細」です。
# $reason は元のエラーの詳細です。
peers-transport = 自宅サーバーに接続できません：{ $reason }
# $seconds はタイムアウトの秒数です。
peers-timeout = { $seconds } 秒以内に応答がありませんでした
# 接続情報やローカルプロキシがまだなく、リクエストを送れません。
peers-not-connected = まだ自宅サーバーに接続していません。
# シェルがサーバーペアリング以外の操作をここに送ってきました。
peers-invalid-action = サーバーのペアリング操作ではありません。

# $reason はローカルプロキシや HTTP クライアントの元のエラーです。
peers-invalid-proxy = ローカルプロキシを利用できません：{ $reason }
# $status は HTTP ステータスコードです。
peers-server-status = 自宅サーバーが HTTP { $status } を返しました
# $reason は JSON パーサーの元のエラーです。
peers-invalid-response = 自宅サーバーの応答を読み取れません：{ $reason }
friends-transport = 自宅サーバーに接続できません：{ $reason }
friends-server-status = 自宅サーバーが HTTP { $status } を返しました
friends-invalid-response = 自宅サーバーの応答を読み取れません：{ $reason }
friends-not-signed-in = まだこのアカウントにサインインしていません。
friends-not-connected = まだ自宅サーバーに接続していません。
friends-invalid-action = 友だちの操作ではありません。
friends-local-open = 端末内の友だちリストを開けません：{ $reason }
friends-invalid-proxy = ローカルプロキシを利用できません：{ $reason }
friends-http-client = ローカル HTTP クライアントを作成できません：{ $reason }
friends-invalid-url = 友だちサービスのアドレスが無効です：{ $reason }
friends-request-body = 友だちリクエストをエンコードできません：{ $reason }
friends-timeout = 友だちサービスから時間内に応答がありませんでした。
friends-group-name-required = グループ名を入力してください。
friends-group-not-found = グループ { $id } が見つかりません。
# $reason はトンネルに拒否されたときの元の文章です。
peers-rejected = 自宅サーバーがこのデバイスを拒否しました：{ $reason }

# セットアップの各ステップ。操作が違うステップで行われたり、状態を取得できていないときの理由です。
onboarding-steps-not-in-flow = セットアップが進行中ではありません
onboarding-steps-already-committed = すでに完了しています
onboarding-steps-not-profile-step = セットアップはプロフィールのステップではありません
onboarding-steps-not-agent-step = セットアップはエージェントのステップではありません
onboarding-steps-state-missing = セットアップの状態をまだ取得していません
onboarding-steps-agent-not-listed = そのエージェントは選択肢の一覧にありません
onboarding-steps-agent-not-installable = そのエージェントはインストールできません
onboarding-steps-not-apps-step = セットアップはアプリのステップではありません
onboarding-steps-need-not-listed = その項目はおすすめの一覧にありません
onboarding-steps-committing = 送信中です
onboarding-steps-no-installable-agent = インストールできるエージェントが選ばれていません
onboarding-steps-testing = テストが実行中です
onboarding-steps-model-not-chosen = モデルがまだ選ばれていません
onboarding-steps-cannot-skip = このステップはスキップできません
onboarding-steps-cannot-go-back = このステップからは戻れません
# シェルからコアの知らないセットアップ操作が渡されたときに表示します。
onboarding-invalid-action = セットアップ操作ではありません

# モデルプロバイダーの接続です。
onboarding-model-key-required = APIキーが空です
onboarding-model-not-model-step = セットアップはモデルのステップではありません
onboarding-model-testing = すでにテストが実行中です
onboarding-model-providers-missing = プロバイダーの一覧をまだ取得していません
onboarding-model-provider-not-listed = そのプロバイダーは一覧にありません
onboarding-model-custom-unsupported = モデルゲートウェイはカスタムプロバイダーに対応していません
onboarding-model-name-required = モデル名が空です
onboarding-model-base-url-invalid = Base URLはhttps://…の形式のアドレスである必要があります
onboarding-model-step-or-testing = セットアップはモデルのステップではないか、テストが実行中です
onboarding-model-none-configured = モデルゲートウェイに接続済みのモデルがありません
onboarding-model-state-missing = セットアップの状態をまだ取得していません
onboarding-model-login-missing = 選択したエージェントにそのログイン方式がありません

# 最後のページです。ブロックされた項目はプレビューが更新されるまでこの説明を表示します。
onboarding-review-permissions-changed = 要求された権限が確認時と変わりました。アプリのページから再インストールしてください
# $installed と $listed はインストール元と一覧のアプリのIDで、翻訳しません。
onboarding-review-source-mismatch = インストール元のアプリは{ $installed }で、一覧の{ $listed }と一致しません
onboarding-review-items-not-ready = 準備ができていない項目があります
onboarding-review-item-missing = 最後のページにその項目はありません
onboarding-review-no-failure = その項目は失敗していません

## chat-a フラグメント：crates/wonton-client/src/chat/{login,sign_in,recovery}.rs。
## チャットの問題・エラーの折りたたまれた「詳細」です。CoreError の理由と ChatProblem の message です。

# サインイン関連。$reason は Matrix SDK の元のエラーの詳細です。
chat-sign-in-restore-failed = Matrixセッションを復元できません：{ $reason }
chat-sign-in-no-connection-info = 自宅サーバーの接続情報がまだありません
chat-sign-in-login-failed = Matrixにログインできません：{ $reason }
chat-sign-in-no-saved-session = ログイン後にログイン状態を取得できません

# ログイントークンの取得リクエスト。$reason は上流の原文、または翻訳されたタイムアウトの詳細です。
# $reason はプロキシURLパーサーの元のエラーです。
chat-login-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }
# $reason はHTTPクライアントの元のエラーです。
chat-login-http-client = HTTPクライアントを作成できません：{ $reason }
# $seconds はリクエストのタイムアウト秒数です。
chat-login-timeout = { $seconds }秒以内に応答がありませんでした
chat-login-request-failed = 自宅サーバーからMatrixログイントークンを取得できません：{ $reason }
# $reason はHTTPレスポンスの読み取りの元のエラーです。
chat-login-response-failed = 自宅サーバーからの応答を読み取れません：{ $reason }
# $reason はJSONパーサーの元のエラーです。
chat-login-invalid-token = 自宅サーバーから返されたログイントークンの形式が正しくありません：{ $reason }
# $login_type は自宅サーバーが返したログイン方式、$expected は要求されたログイン方式です。
chat-login-wrong-type = 自宅サーバーが返したログイン方式{ $login_type }は{ $expected }ではありません
# $status は元のHTTPステータスコードです。
chat-login-server-status = 自宅サーバーからHTTP { $status }が返されました
# $message と $kind はサーバーの元のエラーメッセージとエラーコードです。
chat-login-server-error = { $message }（{ $kind }）

# 復旧関連。
chat-recovery-not-connected = 自宅サーバーにまだ接続していません
chat-recovery-reset-no-response = 自宅サーバーが応答しません。バックアップは変更していません。後でもう一度お試しください。
chat-recovery-empty-key = 復旧キーが空です
# $reason はストレージの元のエラーの詳細です。
chat-recovery-key-not-saved = 復旧キーをこのデバイスに保存できませんでした。今書き写してください：{ $reason }
# 回答や時間枠の停止が失敗したときに、確認カードの下の問題の詳細に表示される文章です。
# $message は自宅サーバーの元のエラーメッセージ、または翻訳された詳細です。
chat-confirmation-answer-failed = 回答できませんでした：{ $message }
# $error はHTTPクライアントの元のエラーです。
chat-confirmation-answer-lost = 自宅サーバーからの応答がなく、回答が届いたかはっきりしません。もう一度試せます（{ $error }）
# $message は自宅サーバーの元のエラーメッセージ、または翻訳された詳細です。
chat-confirmation-stop-failed = 停止できませんでした：{ $message }
# $error はHTTPクライアントの元のエラーです。
chat-confirmation-stop-lost = 自宅サーバーからの応答がありませんでした。もう一度試せます（{ $error }）
# $error はJSONパーサーの元のエラーです。
chat-confirmation-response-unreadable = 自宅サーバーの応答を読み取れません（{ $error }）
# $status は元のHTTPステータスコードです。
chat-confirmation-core-status = 自宅サーバーがHTTP { $status }を返しました
# 個人チャットを始めるとき、ユーザー名が空の場合に表示します。
chat-contacts-empty-username = ユーザー名を入力してください。
# $reason はMatrix IDパーサーの元のエラーです。
chat-contacts-invalid-user-id = ユーザーIDが無効です：{ $reason }

# Siri、ショートカット、チャット画面でメッセージを送るとき、本文が空の場合に表示します。
chat-empty-message = メッセージを入力してください。
# Siriやショートカットから渡されたボットのIDを解析できない場合に表示します。
chat-ask-invalid-bot-id = デフォルトのエージェントのIDの形式が正しくありません。
# 既存のダイレクトチャットのルームIDを解析できない場合に表示します。
chat-ask-invalid-room-id = ダイレクトチャットのルームIDが無効です。
chat-layout-invalid-bot-id = Matrix IDの形式が正しくありません（@name:server の形式）。
chat-layout-change-failed = チャットの表示方法を変更できませんでした：{ $reason }

# Matrixがこのデバイスを取り消し、アカウントが失敗扱いになったときに表示します。
chat-session-device-revoked = このデバイスはMatrixで取り消されました。もう一度自宅サーバーとペアリングしてください。

# シェルからコアの知らないチャット操作が渡されたときに表示します。
chat-invalid-action = チャット操作ではありません
# ダイレクトチャットの開始時に、@名前:サーバーの形でないIDが渡されたときに表示します。
chat-invalid-user-id = ユーザーIDの形式が正しくありません。@名前:サーバーのように入力してください
# $file_type は選択したファイルの元のMIMEタイプです。
chat-not-image-type = 画像ファイルではありません：{ $file_type }
# シェルからコアの知らない回答が渡されたときに表示します。
chat-unknown-answer = このバージョンのWontonでは、この回答を認識できません
# 自宅サーバーに接続する前に確認へ回答したり停止したりしたときに表示します。
chat-action-not-connected = 自宅サーバーにまだ接続していません
# Matrixにサインインする前にチャット操作が届いたときに表示します。
chat-not-signed-in = Matrixにまだログインしていません
chat-send-no-response = Matrixサーバーが応答しなくなりました
# $error はルームID解析の元のエラーです。
chat-invalid-room-id = ルームIDが正しくありません：{ $error }
# 通知専用のルームに送ろうとしたときに表示します。
chat-read-only-room = このルームは通知専用のため、メッセージを送信できません
# $room は元のルームIDです。
chat-room-not-subscribed = 先にルーム{ $room }のタイムラインを購読してください
# $error はイベント ID パーサーの元のエラー。
chat-invalid-event-id = メッセージのイベントIDが正しくありません：{ $error }
# 転送・取り消しするメッセージのメニューにその項目がない（取り消し済み、未送信、権限がないなど）ときに表示。
chat-message-action-unavailable = このメッセージにはその操作ができません
# 転送先のルームが存在しないか、まだ参加していないときに表示。
chat-forward-target-not-joined = 転送先のルームにまだ参加していません
# デフォルトのエージェントを選ぶ前に問い合わせたときに表示します。
chat-no-default-agent = デフォルトのエージェントがまだありません。設定で選んでください

# バックグラウンドのチャット操作が失敗したときの問題表示です。$error はMatrix SDKまたは自宅サーバーの元のエラーです。
chat-op-ask-agent-failed = デフォルトのエージェントに質問できませんでした：{ $error }
chat-op-display-name-failed = 表示名を同期できませんでした：{ $error }
chat-op-join-failed = ルームに参加できませんでした：{ $error }
chat-op-leave-failed = ルームから退出できませんでした：{ $error }
chat-op-resend-failed = メッセージを再送できませんでした：{ $error }
chat-op-cancel-send-failed = 送信をキャンセルできませんでした：{ $error }
chat-op-paginate-failed = 過去のメッセージを読み込めませんでした：{ $error }
chat-op-mark-read-failed = ルームを既読にできませんでした：{ $error }
chat-op-direct-chat-failed = ダイレクトチャットを開始できませんでした：{ $error }
chat-op-send-text-failed = メッセージを送信できませんでした：{ $error }
chat-op-send-image-failed = 画像を送信できませんでした：{ $error }
chat-op-forward-failed = メッセージを転送できませんでした：{ $error }
chat-op-redact-failed = メッセージを取り消せませんでした：{ $error }
## ファイル転送と再生の詳細情報。転送リストの項目、ファイルカード、再生失敗の説明に表示されます。

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
# $error はJSONパーサーの元のエラーです。
home-ticket-invalid-json = 接続情報が有効なJSONではありません：{ $error }
# 引数なし。スキャンした接続情報にサーバーへ到達できるアドレスがありません。
home-ticket-no-addresses = 接続情報にサーバーのリレーアドレスも直接接続アドレスもありません
home-ticket-relays-invalid = リレーテーブルが有効ではありません：{ $reason }
# $domain は接続情報に書かれた元のベースドメインです。
home-ticket-invalid-domain = ベースドメインが無効です：{ $domain }
# $error はURLパーサーの元のエラーです。
home-ticket-invalid-web-url = アプリのウェブアドレスが無効です：{ $error }
# $domain は接続情報のベースドメイン、$url は接続情報に書かれた元のアドレスです。
home-ticket-web-url-under-domain = アプリのウェブアドレスは{ $domain }の下のhttp(s)アドレスである必要があります：{ $url }
# $error はURLパーサーの元のエラーです。
home-ticket-invalid-core-url = 自宅サーバーのアドレスが無効です：{ $error }
# $domain は接続情報のベースドメイン、$url は接続情報に書かれた元のアドレスです。
home-ticket-core-url-under-domain = 自宅サーバーのアドレスは{ $domain }のhttp(s)アドレスである必要があります：{ $url }
# $error はネットワーク設定の元のエラーです。
# 引数なし。ペアリング画面のデバイス名欄が空です。
home-pairing-name-empty = デバイス名が空です
home-pairing-bind-failed = ネットワーク接続を確立できません：{ $error }
# $reason は接続情報の解析の元のエラーです。
home-ticket-corrupt = 保存された接続情報が壊れています：{ $reason }
home-no-connection-info = 自宅サーバーの接続情報がまだありません
# $error はローカルプロキシの元のバインドエラーです。
home-proxy-start-failed = ローカルプロキシを開始できませんでした：{ $error }
home-tunnel-closed = トンネルクライアントが閉じられました
home-lan-unsupported = このバージョンのクライアントコアはローカルネットワーク設定に対応していません
home-port-mapping-save-failed = ポートマッピングの設定を保存できませんでした：{ $error }
# タブバー（tabs/view.rs、tabs/config.rs）と外観（appearance/mod.rs）の画面の「詳細」。
# コアの文字は両シェルで共通のため Android の書き方に合わせ、ナビゲーションバーと書きます。プラグインはユーザーには「アプリ」です。

# ナビゲーションバー ViewModel のエラー。CoreError の理由、またはナビゲーションバーの問題表示として出ます。
# 引数はありません。
tabs-view-always-pinned = チャットとアプリはすでにナビゲーションバーに固定されています。
tabs-view-plugin-list-not-ready = アプリの一覧をまだ読み込んでいないため、今はアプリのページを操作できません。
tabs-view-unknown-plugin = このアプリが見つかりません。
# $error はローカル保存の元のエラー。
tabs-storage-problem = タブバーの設定を保存できません。次回の起動では古い状態に戻ることがあります: { $error }
tabs-view-plugin-no-page = このアプリには開けるウェブページがありません。
tabs-page-not-placed = このページはパネルにありません。

# ナビゲーションバーの設定ファイル（tabs.json）のエラー。CoreError の理由として出ます。
# $path は設定ファイルのパス、$reason は元のパースまたは入出力エラーです。
tabs-config-read-failed = ナビゲーションバーの設定ファイル { $path } を読み込めません：{ $reason }
tabs-config-corrupt = ナビゲーションバーの設定ファイル { $path } が破損しています：{ $reason }
# $count はナビゲーションバーに置ける項目の上限（MAX_ITEMS）です。
tabs-config-too-many-items = ナビゲーションバーには最大 { $count } 個までしか項目を追加できません。
tabs-config-duplicate-page = ナビゲーションバーに重複したページがあります。
tabs-config-plugin-page-incomplete = アプリのページにタイトルとアイコンがありません。
tabs-config-missing-fixed = ナビゲーションバーにチャットがありません。
# $path は設定ファイルのパス、$reason は元の書き込みエラーです。
tabs-config-write-failed = ナビゲーションバーの設定ファイル { $path } に書き込めません：{ $reason }

# 外観の設定のエラー。appearance-settings-unreadable は外観ページの問題表示（AppearanceProblem.message）として出て、ほかはその中の理由または CoreError::Storage の理由です。
# $path は設定ファイルのパス、$reason は元のパースまたは入出力エラーです。
appearance-settings-unreadable = 外観の設定を読み込めないため、デフォルト値を使用しています：{ $reason }
appearance-settings-read-failed = 外観の設定ファイル { $path } を読み込めません：{ $reason }
appearance-settings-corrupt = 外観の設定ファイル { $path } が破損しています：{ $reason }
# $value は設定ファイルにあった元の色の文字列で、変更しません。
appearance-settings-invalid-accent = 外観の設定ファイル { $path } の色が無効です：{ $value }
# $path は設定ファイルのパス、$reason は元の書き込みエラーです。
appearance-settings-write-failed = 外観の設定ファイル { $path } に書き込めません：{ $reason }

## プラグインモジュールの技術的な詳細です。エラー、アプリのタスク、プロキシ・ダウンロードミラー設定ページの「詳細」に折りたたまれて表示されます。
## 長さの制限はありません。引数は診断データで、翻訳キーではありません。

# プラグイン管理API（crates/wonton-client/src/plugins/api.rs）。
# $reason は元のエラーの詳細です。
plugins-api-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }
plugins-api-http-client = HTTPクライアントを作成できません：{ $reason }
# $reason はJSONパーサーの元のエラーです。
plugins-api-invalid-response = 自宅サーバーから返された内容の形式が正しくありません：{ $reason }
# $reason はJSONシリアライザーの元のエラーです。
plugins-api-invalid-request = リクエストボディを書き出せません：{ $reason }
# $reason は拒否された画像のアドレス、またはURLパーサーの元のエラーです。
plugins-api-not-image-url = アプリ画像のアドレスではありません：{ $reason }
# $status は元のHTTPステータスコードです。
plugins-api-server-status = 自宅サーバーからHTTP { $status }が返されました
plugins-api-not-connected = 自宅サーバーに接続してからもう一度お試しください。
plugins-api-server-too-old = 自宅サーバー { $version } は古すぎます。この機能には { $minimum } 以降が必要です。

# インストール・アップデート・アンインストールのタスク（crates/wonton-client/src/plugins/tasks.rs）。
plugins-tasks-uncertain = 接続が切断されたため、自宅サーバーでの結果は不明です。インストール済みリストを確認してください。
plugins-tasks-server-changed = 自宅サーバーが変わったため、以前のサーバーでの結果は不明です。
plugins-tasks-preview-no-source = このプレビューには提供元がないため、インストールできません。
# $id はそのままのタスクIDです。
plugins-tasks-no-such-task = そのようなタスクはありません：{ $id }
plugins-tasks-task-not-done = タスクがまだ完了していません。
# $title はアプリのタイトルです。ストアの表記のまま表示されます。
plugins-tasks-task-running = { $title }はすでにインストール・アップデート・アンインストールの途中です。

# プロキシ設定（crates/wonton-client/src/plugins/egress.rs）。
plugins-egress-save-in-progress = 前回の変更がまだ保存中です。
plugins-egress-not-loaded = プロキシの設定がまだ取得できていません。
plugins-egress-no-confirmation = 確認待ちの変更はありません。
plugins-egress-env-locked = プロキシはWONTON_EGRESS_PROXYで指定されているため、ここでは変更できません。
plugins-egress-plugin-and-address = プロキシアプリを使う場合、プロキシのアドレスは同時に入力できません。
plugins-egress-plugin-not-installed = プロキシアプリがまだインストールされていません。
# $reason は接続問題の元の詳細です。
plugins-egress-save-uncertain = 変更が保存できたかはっきりしません。接続後に確認してください：{ $reason }

# ダウンロードミラーの設定（crates/wonton-client/src/plugins/registry_mirror.rs）。
plugins-registry-save-in-progress = 前回のダウンロードミラーの変更がまだ保存中です。
plugins-registry-not-loaded = ダウンロードミラーの設定がまだ取得できていません。
# $reason は接続問題の元の詳細です。
plugins-registry-save-uncertain = 変更が保存できたかはっきりしません。接続後に設定をもう一度読み込んでください：{ $reason }

# 複数のサーバー（crates/wonton-client/src/servers.rs、client/push_route.rs）。
servers-removed = このサーバーはこのスマートフォンから削除されています。
servers-push-from-removed = このメッセージを送ったサーバーはこのスマートフォンから削除されています。
servers-push-scope-closed = そのサーバーとの接続はすでに終了しています。
# $seconds は登録を待った秒数です。
servers-push-register-timeout = { $seconds }秒以内に通知を登録できませんでした。

chat-avatar-cache-not-file = アバターのキャッシュのパスがファイルではありません
chat-avatar-cache-hash-collision = アバターのキャッシュでハッシュが衝突しています
chat-avatar-cache-missing-mxc = アバターのキャッシュにMXCアドレスがありません
chat-avatar-cache-path-not-utf8 = アバターのキャッシュのパスが有効なUTF-8ではありません
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

# 接続のローカルストレージ（crates/wonton-client/src/home/storage.rs）。
# $path はファイルのパス、$reason は元のパースまたは入出力エラーです。
home-storage-device-key-corrupt = デバイスキーのファイル{ $path }が破損しています：{ $reason }
home-storage-io-failed = { $path }を読み書きできません：{ $reason }

# チャットのローカルストレージ（crates/wonton-client/src/chat/storage.rs）。
# $path はファイルのパス、$reason は元のパースまたは入出力エラーです。
chat-storage-session-corrupt = チャットのログイン状態のファイル{ $path }が破損しています：{ $reason }
chat-storage-io-failed = { $path }を読み書きできません：{ $reason }

# 登録済みサーバーのリスト（crates/wonton-client/src/servers.rs）。
# $path はサーバーリストファイルのパス、$reason は元のパースまたは入出力エラーです。
servers-list-corrupt = サーバーリストのファイル{ $path }が破損しています：{ $reason }
servers-add-current-unpaired = このサーバーを設定してから、別のサーバーを追加してください
servers-name-required = 名前を入力してください
# $path は読み書きまたは削除の対象のパス、$reason は元の入出力エラーです。
servers-storage-failed = { $path }を読み書きできません：{ $reason }
# シェルからコアの知らないサーバー一覧の操作が渡されたときに表示します。
servers-invalid-action = サーバー一覧の操作ではありません

# セッションリスト（crates/wonton-client/src/session_store.rs）。
sessions-title-required = セッションのタイトルを入力してください
# $id はそのままのセッションIDです。
sessions-not-found = そのセッションはありません：{ $id }
# シェルからコアの知らないセッション操作が渡されたときに表示します。
sessions-invalid-action = セッション操作ではありません

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

# 自宅サーバー上のアプリの起動（crates/wonton-client/src/plugins/start.rs）。
# $id はそのままのアプリIDです。
plugins-start-busy = { $id }はすでに起動、インストール、更新、またはアンインストールの実行中です

# アプリページのブリッジ（crates/wonton-client/src/tabs/bridge/）。
tabs-bridge-page-not-open = このアプリのページは開かれていないか、固定されていません
tabs-bridge-invalid-environment = ページのインセットとセーフエリアは有限の非負の数である必要があります
tabs-bridge-public-page = 公開ウェブページには Wonton のブリッジがありません。

# チャットの接続問題（crates/wonton-client/src/chat/view.rs）：CoreError::TransportUnavailableの理由と失敗した操作のメッセージ。
chat-not-connected = 自宅サーバーに接続してから、もう一度試してください
chat-draft-save-failed = 下書きの保存に失敗しました：{ $error }
chat-draft-clear-failed = 送信済みの下書きを消すのに失敗しました：{ $error }
chat-draft-not-signed-in = まだ Matrix にログインしていません
chat-draft-read-interrupted = 下書きの読み込みが中断されました

# セットアップのローカル記録（crates/wonton-client/src/onboarding/storage.rs）。
# $path はファイルのパス、$reason は元の入出力エラーです。
onboarding-storage-io-failed = { $path }を読み書きできません：{ $reason }

# 自宅サーバーのワンクリックアップデート（crates/wonton-client/src/plugins/core_upgrade.rs）：不確定な段階の説明と拒否された操作で、アップデートページやエラーの「詳細」に表示されます。
# 引数なし。自宅サーバーがこのアプリでは理解できない結果を返しました。
plugins-core-upgrade-unrecognized = 自宅サーバーが、このバージョンのWontonでは理解できない結果を返しました。Wontonを更新してからもう一度確認してください
# 実行中のダイジェストが欠けている場合の代わりの語。文ではありません。
plugins-core-upgrade-unknown-digest = 不明
# 自宅サーバーはアップデート完了を報告していますが、実行中のバージョンが目標と一致しません。$running は実行中のダイジェストか「不明」、$target はそのままの目標ダイジェストです。
plugins-core-upgrade-completed-mismatch = 自宅サーバーはアップデートが完了したと報告していますが、実行中のバージョンが目標と異なります：実行中 { $running }、目標 { $target }
# 制限時間内に接続できず、結果は不明です。$digest は旧バージョンに戻すときに固定するそのままのダイジェストです。
plugins-core-upgrade-timed-out = 制限時間を過ぎても自宅サーバーに接続できず、アップデートの結果は不明です。接続するとここが更新されます
plugins-core-upgrade-timed-out-rollback = 制限時間を過ぎても自宅サーバーに接続できず、アップデートの結果は不明です。接続するとここが更新されます。旧バージョンに戻すには、ホストでインストールコマンドをもう一度実行し、WONTON_CORE_PIN={ $digest } を付けてください
plugins-core-upgrade-invalid-action = 自宅サーバーのアップデートの操作ではありません
plugins-core-upgrade-already-running = すでにアップデートを実行中です
plugins-core-upgrade-not-cancellable = 今はキャンセルできません
# $reason は接続問題の元の詳細です。
plugins-core-upgrade-start-uncertain = アップデートを開始できたかはっきりしません。接続後にもう一度確認してください：{ $reason }

# アップデート前の互換性チェック（ADR-0085）の詳細：理由ごとに 1 行、{ $raw } はマニフェ
# ストの原文の値。クライアントコアがアプリの言語で書き、アップデートページでは新しいバー
# ジョンが読めないアプリの下に並べます。
plugins-core-upgrade-compat-spec = このアプリは仕様{ $raw }で書かれており、新しいバージョンは対応していません。アップデート後は異常として表示されます。
plugins-core-upgrade-compat-feature = このアプリのマニフェストには、新しいバージョンが認識しない{ $raw }があります。アップデート後は異常として表示されます。
plugins-core-upgrade-compat-capability = このアプリは能力{ $raw }に依存していますが、新しいバージョンでは提供されません。アップデート後は異常として表示されます。
plugins-core-upgrade-compat-other = 新しいバージョンはこのアプリを読めません。アップデート後は異常として表示されます。
plugins-core-upgrade-compat-other-raw = 新しいバージョンはこのアプリの{ $raw }を読めません。アップデート後は異常として表示されます。

# プロフィールのステップ：名前やメールアドレスを保存できない理由です。$max は名前の最大文字数です。
onboarding-profile-name-empty = 名前を入力してください
onboarding-profile-name-too-long = 名前は{ $max }文字以内にしてください
onboarding-profile-name-control = 名前に改行や制御文字は使えません
onboarding-profile-email-invalid = メールアドレスの形式ではありません。me@example.comのように、空白や制御文字を含めずに入力してください
onboarding-profile-email-cleared = 一度設定したメールアドレスは変更できますが、削除はできません。メールアドレスが必要なアプリの再構築に使われます

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

# ペアリングの詳細。アドレスとリレー URL にペアリングキーは含まれません。error は上流の原文です。
home-pairing-timeout = { $seconds } 秒以内にペアリングを完了できませんでした。
home-pairing-aborted = ペアリングが完了する前に中断しました。もう一度お試しください。
home-pairing-not-pairing = 進行中のペアリングはありません。
home-pairing-failed-detail = 直接接続先：{ $addresses }。リレー：[{ $relays }]。経過時間：{ $milliseconds } ミリ秒、制限時間：{ $seconds } 秒。原因：{ $error }
plugin-now-start-failed = ワンタップ操作を開始できません：{ $error }

# パネル（ADR-0052）のエラー。CoreError の理由、または TabsProblem の説明として表示する。
panel-full = パネルがいっぱいです。先にページを外してください。
panel-pins-not-connected = まだ自宅サーバーに接続していません。
panel-pins-api-failed = { $status }：{ $body }

panel-page-already-there = このページはすでにパネルにあります。
panel-page-not-there = このページはパネルにありません。
panel-page-unsupported = パネルに入れられるのはアプリのページ、アプリ内のページ、https:// のウェブページだけです。
panel-index-out-of-range = その位置はパネルの外です。
panel-not-signed-in = まだログインしていないため、パネルの変更をアカウントに保存できませんでした。端末には記録してあります。
panel-room-not-open = このチャットのパネルを変えるには、先に開いてください。
panel-room-unknown = このチャットが見つかりません。
panel-pin-invalid-url = そのアドレスは読み取れません。
panel-pin-not-https = ピンできるのは https:// のウェブページだけです。
panel-pin-path-outside = そのページはアプリ自身のサイトの外です。
panel-attention-panel-only = パネル内のページだけに注意の印を付けられます。
panel-config-too-many = パネルには最大 { $count } ページまで入ります。
panel-config-duplicate = 同じページがパネルに二つあります。
panel-sync-failed-read = サーバーからパネルの設定を読み取れませんでした。
panel-sync-unreadable = サーバーのパネル設定をこの端末では読めないため、端末のものを使います。
tabs-page-cannot-reload = このページは再読み込みできません。

tabs-bar-not-pinned = このページはナビゲーションバーに固定されていません。
tabs-bar-already-pinned = このページはすでにナビゲーションバーに固定されています。
tabs-bar-replace-required = ナビゲーションバーがいっぱいです。置き換えるページを指定してください。
tabs-bar-replacing-missing = 置き換えるページがナビゲーションバーにありません。
tabs-bar-fixed = チャットとアプリは置き換えられません。
tabs-bar-not-full = ナビゲーションバーがいっぱいではないので、置き換えるページは指定できません。
tabs-bar-remove-fixed = チャットとアプリは外せません。
tabs-bar-page-missing = このページはナビゲーションバーにありません。
tabs-bar-index-out-of-range = その位置はナビゲーションバーの範囲外です。
tabs-open-only-plugins = 開き方を設定できるのはアプリのページだけです。
tabs-fullscreen-only-plugins = 全画面にできるのはアプリのページだけです。
tabs-fullscreen-none = 全画面のページはありません。
tabs-fullscreen-not-current = そのページは全画面のページではありません。
panel-pins-refresh-failed = Bot がピン留めしたページを読み込めません:{ $error }
