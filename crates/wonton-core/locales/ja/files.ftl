## ファイルサービスがApp・ボット・エージェントに送るテキスト（ADR-0043）。完全な文を 1 メッセージにし、
## コードでの組み立てはしない。データ（ファイルID、サイズ、プラグインID、上流から返された理由）は
## パラメーターとして文に埋め込み、翻訳しない。説明はです・ます調。

files-not-found = そのファイルはありません
files-moved-or-deleted = ファイルは移動されたか、削除されました
files-not-shared = このファイルはあなたと共有されていません
files-share-expired = このファイルIDの共有は期限切れです。ファイルをもう一度取得してください
files-changed = ファイルは登録後に変更されたため、表示した内容が現在のものとは違う可能性があります。もう一度取得してください
files-share-not-authorized = アプリ{ $plugin }には共有フォルダー{ $share }の権限がありません
files-mcp-gone = アプリ{ $plugin }は実行中ですが、MCPサービスを提供しなくなったため、このリソースを読み取れません
files-plugin-unavailable = 元のアプリ{ $plugin }がアンインストールされたか、起動していません
files-notify-no-matrix = 自宅サーバーはMatrix（WONTON_MATRIX_URL）に接続していないため、「Wonton」の個人チャットに送れません
files-matrix-send-failed = 「Wonton」の個人チャットに送れません：{ $error }
files-upstream-not-own = { $service }はアプリ{ $plugin }自身のサービスではありません

# プレビューとサムネイル。
files-too-big-for-thumbnail = ファイルが{ $limit } MiBを超えるため、サムネイルは作成しません
files-pdf-unreadable = このPDFは解析できません（暗号化されているか、壊れています）。内容は読めません
files-pdf-no-text = このPDFにはテキスト層がありません（スキャン画像か図版のみです）。全{ $pages }ページで、図表の中身は読めません
files-too-big-for-parsing = ファイルが{ $limit } MiBを超えるため、内容を抽出できません
files-video-unsupported = この動画のコンテナは読めません（MP4・M4V・MOVのみ対応）。ダウンロードか共有だけができます
files-video-unreadable = この動画は解析できません（コンテナか構造が不明です）。長さと解像度を取得できません
files-audio-unreadable = 音声は内容を読めません。ダウンロードか共有だけができます
files-thumbnail-timeout = 元のファイルを{ $after }以内に読み終えられませんでした
files-idle-limited = アプリからのレスポンスは{ $idle }間、新しいバイトがありません
files-declared-size-over-limit = アプリが宣言したサイズがresources/readの上限（{ $limit } MiB）を超えています
files-not-text-nul = { $name }はテキストファイルではありません（内容にNULバイトがあります）。PDFなどのバイナリーファイルは、ダウンロードか共有だけができます。
files-not-text-utf8 = { $name }はテキストファイルではありません（内容がUTF-8として解釈できません）。PDFなどのバイナリーファイルは、ダウンロードか共有だけができます。
files-no-preview = この種類にはプレビューもサムネイルもありません
files-preview-failed = プレビューもサムネイルも作成できません：{ $why }
files-decode-busy = 画像のデコードが混み合っています
files-view-busy = このファイルは読み取り中です。しばらくしてから再試行してください
files-read-busy = 画像の読み取りが混み合っています

# 保存・アップロード・削除。
files-stage-no-space = { $total }バイトを書き込む余地がありません。ディスクには5%の空きが必要です
files-delete-transfers-only = 自宅サーバーが削除できるのは、共有フォルダー{ $folder }内のファイルだけです
files-size-mismatch = 転送されたデータは{ $actual }バイトですが、作成時に宣言されたのは{ $size }バイトです
files-no-hard-links = { $name }：共有フォルダー{ $folder }のファイルシステムはハードリンクに対応していません
files-name-taken = { $preferred }には同名のファイルが{ $count }個あります
files-short-read = { $copied }バイトしか読み取れませんでした。{ $size }バイトのはずです
files-upload-disk-full = ディスクにこの{ $size }バイトのファイルを置けません（{ $percent }%の空きが必要で、inodeも消費します）
files-upload-leftover = ディスクの残りは{ $free }バイトで、このアップロードにはあと{ $left }バイト必要です
files-upload-quota-full = ディスクのクォータが一杯で、このアップロードは収まりません
files-upload-inodes-full = ディスクの inode を使い切っていて、このアップロードは収まりません
files-upload-record-label = 未完了のアップロード記録
files-upload-record-unwritable = 未完了のアップロード記録を書き込めなかったため、ファイルの配置は保留しました
files-path-collision = このパスには別のファイルがあります：IDは削除しましたが、ファイル自体はそのままです
files-residual-undeletable = このアップロードの残りを清理できなかったため、ファイルは削除していません

# アプリリソースを読むためのスロット。
files-plugin-read-overrun = アプリ{ $plugin }の読み取りが{ $seconds }秒を超えても終わりません
files-slots-full = アプリ{ $plugin }の読み取りがいっぱいです。しばらくしてからもう一度お試しください
files-plugin-slots-exhausted = アプリリソースの読み取りスロットがすべて使用中です

# アプリからリソースを読む。
files-resource-invalid-url = 内部アドレスが不正です：{ $error }
files-resource-http-only = 内部アドレスにはhttpだけ使えます
files-resource-no-credentials = 内部アドレスにユーザー名やパスワードは含められません
files-resource-host = 内部アドレスのホスト名はアプリのサービス名である必要があり、IPアドレスは使えません
files-resource-unreachable = アプリのサービスに接続できません：{ $why }
files-resource-redirect = アプリの内部アドレスがリダイレクト（{ $status }）を返しました。自宅サーバーは追従しません
files-upstream-status = アプリが{ $status }を返しました：{ $detail }
files-upstream-read-failed = アプリのレスポンスを読み取れません：{ $error }

# ファイル削除用の退避スロット。
files-trash-foreign-slot = 退避スロット{ $name }は自宅サーバーが作ったものではありません（所有者{ $owner }、モード{ $mode }、期待値は{ $euid }、700）。削除するには自宅サーバーを再起動してください
files-trash-placeholder-missing = 退避スロット{ $index }のプレースホルダーファイルが見つかりません。削除するには自宅サーバーを再起動してください
files-trash-full = { $folder }の退避スロットがすべて使用中です。中のファイルを戻してから削除するには、自宅サーバーを再起動してください

# 共有フォルダ。
files-check-share = 共有フォルダー{ $share }がありません。wonton__list_sharesで一覧を確認してください

# ゲートウェイのファイルAPI。
files-service-disabled = 自宅サーバーでファイルサービスが有効になっていません
files-device-only = ファイルAPIはトンネル経由のペアリング済みデバイス専用で、{ $header }: 1が必要です
files-stream-shortened = ダウンロード中にファイルが短くなりました
files-invalid-room-body = リクエストボディは{"{"}"room": …{"}"}の形式にしてください：{ $error }

# tusd経由のアップロード。
files-tusd-unconfigured = 自宅サーバーにtusdが設定されていません（WONTON_TUSD_URL）
files-tusd-required = アップロードAPIにはTus-Resumable: 1.0.0が必要です
files-tusd-timeout = tusdからの応答がありません。しばらくしてからもう一度お試しください
files-tusd-gone = tusdにこのアップロードのデータはもうありません。もう一度アップロードしてください
files-upload-disconnected = このアップロードを作成したリクエストは既に切断されています
files-upload-length-required = アップロードは作成時に長さを宣言する必要があります（Upload-Length）
files-upload-filename-required = アップロードはUpload-Metadataにfilenameを含める必要があります
files-upload-finished = このアップロードは既に完了しています。ファイルを削除するにはDELETE /_wonton/files/{ $id }を使ってください
files-upload-other-device = このアップロードは別のデバイスが作成しました
files-upload-stale = このアップロードは長い間進捗がなく削除されました。もう一度アップロードしてください
files-hook-tusd-only = ファイルフックはtusd専用です
files-hook-not-from-gateway = このリクエストはゲートウェイを経由していません
