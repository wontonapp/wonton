import Foundation
import Network

/// 界面测试用的本地 Matrix homeserver 加授权服务器（只在测试里）：只有 OAuth 2.0 登录（MSC2965），动态注册永远成功，
/// 授权页上只有一个链接，点了交回 state 对不上的回调。测的是壳和核心怎么走取消、回调对不上、重试，不连外网。
/// 只听 127.0.0.1，App 在同一台模拟器里直接连得上。
final class MatrixLoginStub: @unchecked Sendable {
    static let mismatchLink = "Return a mismatched result"

    private let listener: NWListener
    private let queue = DispatchQueue(label: "matrix-login-stub")
    /// 只在 `start` 等到监听就绪之前由监听队列写，之后只读。
    private var port: UInt16 = 0

    var url: String { "http://127.0.0.1:\(port)" }

    init() throws {
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: .ipv4(.loopback), port: .any)
        listener = try NWListener(using: parameters)
    }

    func start() throws {
        let ready = DispatchSemaphore(value: 0)
        listener.newConnectionHandler = { [self] connection in
            connection.start(queue: queue)
            receive(connection, buffer: Data())
        }
        listener.stateUpdateHandler = { [self] state in
            switch state {
            case .ready:
                port = listener.port?.rawValue ?? 0
                ready.signal()
            case .failed, .cancelled:
                ready.signal()
            default:
                break
            }
        }
        listener.start(queue: queue)
        guard ready.wait(timeout: .now() + 10) == .success, port != 0 else {
            throw NSError(domain: "MatrixLoginStub", code: 1, userInfo: [NSLocalizedDescriptionKey: "本地桩没监听起来"])
        }
    }

    func stop() {
        listener.cancel()
    }

    private func receive(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [self] data, _, isComplete, error in
            var buffer = buffer
            if let data { buffer.append(data) }
            if let request = StubRequest(buffer) {
                respond(connection, to: request)
            } else if isComplete || error != nil {
                connection.cancel()
            } else {
                receive(connection, buffer: buffer)
            }
        }
    }

    private func respond(_ connection: NWConnection, to request: StubRequest) {
        let (status, type, body) = route(request)
        let head = "HTTP/1.1 \(status)\r\nContent-Type: \(type)\r\n"
            + "Content-Length: \(body.utf8.count)\r\nConnection: close\r\n\r\n"
        connection.send(content: Data((head + body).utf8), completion: .contentProcessed { _ in connection.cancel() })
    }

    private func route(_ request: StubRequest) -> (String, String, String) {
        let json = "application/json"
        switch (request.method, request.path) {
        case ("GET", "/_matrix/client/versions"):
            return ("200 OK", json, #"{"versions":["v1.11","v1.15"]}"#)
        case ("GET", "/_matrix/client/v3/login"):
            // 只有旧式 SSO：核心只给浏览器登录。
            return ("200 OK", json, #"{"flows":[{"type":"m.login.sso"}]}"#)
        case ("GET", "/_matrix/client/v1/auth_metadata"):
            return ("200 OK", json, metadata)
        case ("POST", "/oauth2/register"):
            return ("201 Created", json, #"{"client_id":"stub-client"}"#)
        case ("GET", "/oauth2/authorize"):
            return ("200 OK", "text/html; charset=utf-8", authorizePage)
        default:
            return ("404 Not Found", json, #"{"errcode":"M_UNRECOGNIZED","error":"not in the stub"}"#)
        }
    }

    private var metadata: String {
        """
        {"issuer":"\(url)/","authorization_endpoint":"\(url)/oauth2/authorize",\
        "token_endpoint":"\(url)/oauth2/token","registration_endpoint":"\(url)/oauth2/register",\
        "revocation_endpoint":"\(url)/oauth2/revoke","response_types_supported":["code"],\
        "response_modes_supported":["query","fragment"],\
        "grant_types_supported":["authorization_code","refresh_token"],\
        "code_challenge_methods_supported":["S256"]}
        """
    }

    private var authorizePage: String {
        """
        <!doctype html><meta name="viewport" content="width=device-width">
        <h1>Stub sign-in</h1>
        <p><a style="font-size:2em" href="io.github.wontonapp:/matrix-login?code=stub&state=not-this-login">\
        \(Self.mismatchLink)</a></p>
        """
    }
}

/// 读够了一个完整的请求（请求头加 Content-Length 那么长的正文）才算数。
private struct StubRequest {
    let method: String
    let path: String

    init?(_ data: Data) {
        guard let end = data.range(of: Data("\r\n\r\n".utf8)),
              let head = String(data: data[..<end.lowerBound], encoding: .utf8)
        else { return nil }
        let lines = head.components(separatedBy: "\r\n")
        let parts = lines.first?.split(separator: " ") ?? []
        guard parts.count >= 2 else { return nil }
        let length = lines.dropFirst()
            .first { $0.lowercased().hasPrefix("content-length:") }
            .flatMap { Int($0.split(separator: ":")[1].trimmingCharacters(in: .whitespaces)) } ?? 0
        guard data.count - end.upperBound >= length else { return nil }
        method = String(parts[0])
        path = String(parts[1].split(separator: "?")[0])
    }
}
