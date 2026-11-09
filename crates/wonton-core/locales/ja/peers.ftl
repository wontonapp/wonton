## サーバー同士のペアリング（ADR-0050）で App とチャットカードに見せる文字（ADR-0043）。

peers-endpoint-detail = endpoint { $id }、前回のエラー：{ $error }
peers-endpoint = endpoint { $id }
peers-request-title = { $owner }のサーバーがあなたのサーバーへの接続を求めています
peers-request-server = 相手のサーバー：{ $server }
peers-connect = 接続
peers-reject = 拒否
peers-connected = 接続しました
peers-error-code-expired = ペアリングコードの有効期限が切れています。新しいコードを発行してください
peers-error-code-invalid = コードを読み取れません：Wontonのサーバー用ペアリングコードではないか、改ざんされています
peers-error-peer-unreachable = 相手のサーバーに接続できません。しばらくしてからもう一度お試しください
peers-error-peer-rejected = 相手がペアリングを拒否しました
peers-error-already-paired = 2台のサーバーはすでにペアリング済みです
peers-error-self-pairing = これは自分のサーバーのコードです
peers-error-not-paired = このサーバーとはまだペアリングしていません
peers-error-unauthenticated = まずWontonでペアリングしてから、Wontonからこのページを開いてください
peers-error-transport = ペアリングが完了しませんでした。しばらくしてからもう一度お試しください
peers-error-matrix-unavailable = このサーバーはMatrixに接続していないため、ペアリングは使えません
peers-error-not-paired-remove = このサーバーとはペアリングしていません
peers-error-code-revoked = このペアリングコードは無効化されています。相手に新しいコードを発行してもらってください
peers-error-too-many-requests = 相手のサーバーが現在ペアリング要求を受け付けられません。しばらくしてからもう一度お試しください
peers-error-request-not-found = このリクエストはすでにありません
peers-new-request = { $owner }のサーバー（{ $server }）があなたのサーバーへの接続を求めています
