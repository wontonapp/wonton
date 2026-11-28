//! 辅助容器：lego 的 httpreq 写入器、DNS 转发（`*.auth.<zone>` 给 acme-dns）和慢下载后端。

use super::*;

/// 辅助容器里的脚本：RAW 模式的 httpreq 写入 challtestsrv、按后缀转发 DNS、慢下载。
const HELPER: &str = r#"
import base64, hashlib, http.client, json, socket, struct, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

chall, acme, zone, apex, lan = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
if not chall.startswith("http"):
    chall = "http://" + chall
chall_host, chall_port = chall.split("://", 1)[1].rsplit(":", 1)
chall_addr = chall.split("://", 1)[1]

txts = {}
lock = threading.Lock()

def name_bytes(name):
    return b"".join(bytes([len(p)]) + p.encode() for p in name.rstrip(".").split(".")) + b"\x00"

def post(path, body):
    connection = http.client.HTTPConnection(chall_host, int(chall_port), timeout=5)
    connection.request("POST", path, body=json.dumps(body), headers={"content-type": "application/json"})
    connection.getresponse().read()
    connection.close()

def txt_values(host):
    labels = host.rstrip(".").split(".")
    question = b"".join(bytes([len(p)]) + p.encode() for p in labels) + b"\x00\x00\x10\x00\x01"
    packet = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00" + question
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(3)
    try:
        sock.sendto(packet, (chall_host_of(acme), 53))
        data = sock.recvfrom(4096)[0]
    except OSError:
        return []
    position = 12
    while data[position]:
        position += 1 + data[position]
    position += 5
    total = struct.unpack("!H", data[6:8])[0]
    values = []
    for _ in range(total):
        while True:
            length = data[position]
            if length & 0xC0 == 0xC0:
                position += 2
                break
            if length == 0:
                position += 1
                break
            position += 1 + length
        rtype, length = struct.unpack("!H", data[position:position + 2])[0], struct.unpack("!H", data[position + 8:position + 10])[0]
        rdata = data[position + 10:position + 10 + length]
        position += 10 + length
        if rtype == 16:
            values.append(rdata[1:].decode())
    return values

def chall_host_of(address):
    return address.rsplit(":", 1)[0]

def cname_answer(data, end, target):
    question = data[12:end + 5]
    rdata = name_bytes(target)
    header = data[:2] + b"\x81\x80" + data[4:6] + struct.pack("!H", 1) + b"\x00\x00\x00\x00"
    answer = b"\xc0\x0c" + struct.pack("!HHIH", 5, 1, 1, len(rdata)) + rdata
    return header + question + answer

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        try:
            self.handle_post()
        except Exception:
            import traceback
            traceback.print_exc()
            self.send_response(500)
            self.end_headers()

    def handle_post(self):
        expected = 'Basic ' + base64.b64encode(b'user$TOKEN:pass\\path$TOKEN "quoted"').decode()
        if self.headers.get("authorization") != expected:
            self.send_response(401)
            self.end_headers()
            return
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", "0"))) or b"{}")
        host = "_acme-challenge." + body.get("domain", "").removeprefix("*.") + "."
        if self.path == "/present":
            value = base64.urlsafe_b64encode(hashlib.sha256(body.get("keyAuth", "").encode()).digest()).rstrip(b"=").decode()
            post("/set-txt", {"host": host, "value": value})
            with lock:
                txts.setdefault(host, []).append(value)
        elif self.path == "/cleanup":
            post("/clear-txt", {"host": host})
            with lock:
                txts.pop(host, None)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")
    def do_GET(self):
        self.send_response(200)
        self.send_header("content-type", "text/plain")
        self.end_headers()
        for index in range(10):
            self.wfile.write(("chunk-%d\n" % index).encode())
            self.wfile.flush()
            time.sleep(0.15)
    def log_message(self, *args):
        pass

def dns():
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", 53))
    while True:
        data, peer = sock.recvfrom(1500)
        end = 12
        labels = []
        while data[end]:
            length = data[end]
            labels.append(data[end + 1:end + 1 + length].decode(errors="ignore"))
            end += 1 + length
        name = ".".join(labels).lower() + "."
        qtype = struct.unpack("!H", data[end + 1:end + 3])[0]
        target_name = None
        if name == "_acme-challenge." + zone + "." and apex != "-":
            target_name = apex
        elif name == "_acme-challenge.lan." + zone + "." and lan != "-":
            target_name = lan
        if target_name:
            values = txt_values(target_name)
            packet = cname_answer(data, end, target_name)
            if qtype == 16 and values:
                question = data[12:end + 5]
                header = data[:2] + b"\x81\x80" + data[4:6] + struct.pack("!H", 1 + len(values)) + b"\x00\x00\x00\x00"
                rdata = name_bytes(target_name)
                answers = b"\xc0\x0c" + struct.pack("!HHIH", 5, 1, 1, len(rdata)) + rdata
                for value in values:
                    encoded = bytes([len(value)]) + value.encode()
                    answers += b"\xc0\x0c" + struct.pack("!HHIH", 16, 1, 1, len(encoded)) + encoded
                packet = header + question + answers
            sock.sendto(packet, peer)
            continue
        if qtype == 16 and name.startswith("_acme-challenge."):
            with lock:
                values = list(txts.get(name, []))
            question = data[12:end + 5]
            header = data[:2] + b"\x81\x80" + data[4:6] + struct.pack("!H", len(values)) + b"\x00\x00\x00\x00"
            answers = b""
            for value in values:
                encoded = bytes([len(value)]) + value.encode()
                answers += b"\xc0\x0c" + struct.pack("!HHIH", 16, 1, 1, len(encoded)) + encoded
            sock.sendto(header + question + answers, peer)
            continue
        target = acme if acme != "-" and name.endswith(".auth." + zone + ".") else chall_addr
        host = target.rsplit(":", 1)[0]
        port = "53"
        upstream = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        upstream.settimeout(5)
        try:
            upstream.sendto(data, (host, int(port)))
            sock.sendto(upstream.recvfrom(4096)[0], peer)
        except OSError as error:
            print("dns error", error, flush=True)

import threading
threading.Thread(target=dns, daemon=True).start()
ThreadingHTTPServer(("0.0.0.0", 8484), Handler).serve_forever()

"#;

pub(super) fn up(case: &mut Case) {
    // 容器到宿主绑的端口不通，所以这些都放进容器。
    let helper_ip = format!("{}.7", case.base);
    let helper_script = case.dir.path().join("helper.py");
    write(&helper_script, HELPER);
    let helper = case.names.oneshot_container("helper");
    start(
        &case.names,
        &case.network,
        &helper,
        None,
        &helper_ip,
        &["entry-gateway.wonton.internal"],
        &[(helper_script, "/helper.py".into())],
        &case.python,
        "python3",
        &[
            "/helper.py",
            &format!("{}:8055", case.challenge_ip),
            case.acme_dns_ip
                .clone()
                .unwrap_or_else(|| "-".into())
                .as_str(),
            ZONE,
            case.acme_dns
                .as_ref()
                .map(|(_, apex, _)| apex.fulldomain.clone())
                .unwrap_or_else(|| "-".into())
                .as_str(),
            case.acme_dns
                .as_ref()
                .map(|(_, _, lan)| lan.fulldomain.clone())
                .unwrap_or_else(|| "-".into())
                .as_str(),
        ],
    );
    case.dns_zone_servers = vec![format!("{helper_ip}:53")];
    case.helper_ip = helper_ip;
    case.helper = helper;
}
