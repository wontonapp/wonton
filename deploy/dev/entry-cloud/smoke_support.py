#!/usr/bin/env python3
"""Local ADR-0051 smoke: real named homes, DNS-01, federation and iroh pairing."""
import html
import http.client
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import tomllib
from urllib.parse import quote
from urllib.request import ProxyHandler, Request, build_opener

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
SLOT = Path.home() / "wonton-dev/stack-slot.sh"
RUNTIME_IMAGE = "public.ecr.aws/docker/library/debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4"
CURL_IMAGE = "curlimages/curl:8.16.0@sha256:463eaf6072688fe96ac64fa623fe73e1dbe25d8ad6c34404a669ad3ce1f104b6"


def run(*args, input=None, env=None, check=True, timeout=180):
    result = subprocess.run(list(map(str, args)), input=input, text=True,
                            capture_output=True, env=env, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"{args[0]} failed ({result.returncode}): {result.stderr}\n{result.stdout}")
    return result


def private(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(0o600)


def free_port(kind=socket.SOCK_STREAM):
    with socket.socket(type=kind) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_for(label, predicate, seconds=60):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.5)
    raise RuntimeError(f"Timed out waiting for {label}")


def curl(network, url, method="GET", body=None, headers=None, proxy=None, ca=None, resolve=None, dns=None):
    options = {"url": url, "request": method, "max-time": "60", "noproxy": "" if proxy else "*"}
    if proxy:
        options.update({"proxy": "http://127.0.0.1:18008", "proxy-user": proxy})
    if ca:
        options["cacert"] = str(ca)
    if resolve:
        options["resolve"] = resolve
    config = "".join(f"{key} = {json.dumps(value)}\n" for key, value in options.items())
    for key, value in (headers or {}).items():
        config += "header = " + json.dumps(f"{key}: {value}") + "\n"
    if body is not None:
        config += "header = \"Content-Type: application/json\"\n"
        config += "data = " + json.dumps(json.dumps(body)) + "\n"
    mounts = ["-v", f"{ca}:{ca}:ro"] if ca else []
    resolver = ["--dns", dns] if dns else []
    reply = run("docker", "run", "--rm", "-i", "--network", network, *mounts, *resolver,
                CURL_IMAGE, "--silent", "--show-error", "--config", "-", "--write-out", "\n%{http_code}",
                input=config, check=False, timeout=90)
    if reply.returncode:
        raise ConnectionError(reply.stderr.strip())
    payload, status = reply.stdout.rsplit("\n", 1)
    return int(status), payload


class Home:
    def __init__(self, label, root, cloud, binary, core_image):
        self.label, self.cloud = label, cloud
        self.project = f"wonton-fr-entry-{label}"
        self.dir = root / label
        self.install, self.data = self.dir / "install", self.dir / "data"
        self.install.mkdir(parents=True)
        self.data.mkdir()
        self.port, self.udp = free_port(), free_port(socket.SOCK_DGRAM)
        self.core = self.project + "-core"
        self.phone = self.project + "-phone"
        self.acquired = False
        self.started = False
        self.proxy = None
        self.binary, self.core_image = binary, core_image

    def compose(self, *args, check=True):
        return run("docker", "compose", "--env-file", self.install / ".env", "-p", self.project,
                   "-f", self.install / "compose.yaml", *args, check=check, timeout=300)

    def install_home(self):
        print(f"Acquiring {self.project}", flush=True)
        # 一台机器的栈上限常被别的任务占满，而家里两套栈只多用一条云网络地址；没有空名额就不登记，直接起。
        free = int(run(SLOT, "free").stdout.strip() or 0)
        if free > 0:
            subprocess.run([str(SLOT), "acquire", "--project", self.project, self.project], check=True)
            self.acquired = True
        else:
            print(f"{self.project}: 没有空栈名额，不登记直接起", flush=True)
        ca = self.cloud["ca"]
        inputs = self.install / "entry-input"
        inputs.mkdir(mode=0o700)
        private(inputs / "invite", run("bash", HERE / "dev.sh", "invite", "3600").stdout.strip())
        shutil.copyfile(ca, inputs / "ca.pem")
        values = {
            "WONTON_DATA_DIR": str(self.data), "WONTON_CLIENT_SERVICE": f"https://names.{self.cloud['zone']}:8443",
            "WONTON_CLIENT_INVITE_FILE": str(inputs / "invite"), "WONTON_CLIENT_CA_FILE": str(inputs / "ca.pem"),
            "WONTON_FRP_CA_FILE": str(inputs / "ca.pem"), "WONTON_ACME_CA_FILE": str(inputs / "ca.pem"),
            "WONTON_ACME_DIRECTORY": f"https://pebble.{self.cloud['zone']}:14000/dir",
            "WONTON_ACME_ACCEPT_TERMS": "true", "WONTON_ACME_NETWORK": self.cloud["network"],
            "WONTON_ACME_RESOLVERS": self.cloud["resolver"] + ":53",
            "WONTON_RELAYS": f"https://iroh.{self.cloud['zone']}",
        }
        private(inputs / "init.env", "".join(f"{key}={value}\n" for key, value in values.items()))
        result = run("docker", "run", "--rm", "--network", self.cloud["network"], "--dns", self.cloud["resolver"],
                     "--user", f"{os.getuid()}:{os.getgid()}", "--group-add", str(os.stat("/var/run/docker.sock").st_gid),
                     "--env-file", inputs / "init.env", "-v", "/var/run/docker.sock:/var/run/docker.sock",
                     "-v", f"{self.dir}:{self.dir}", "-v", f"{self.binary}:/usr/local/bin/wonton-core:ro",
                     self.core_image, "init", self.install, "wonton.localhost", timeout=900)
        self.name = (self.install / "secrets/matrix-server-name").read_text().strip()
        assert self.name.endswith("." + self.cloud["zone"])
        self.registration = json.loads((self.install / "entry/settings.json").read_text())["registration"]
        print(f"Installed {self.label}: {self.name}; real DNS-01 certificate", flush=True)
        env = {"WONTON_CORE_IMAGE": self.core_image, "WONTON_INSTALL_DIR": str(self.install),
               "WONTON_DATA_DIR": str(self.data), "WONTON_BASE_DOMAIN": "wonton.localhost",
               "WONTON_PORT": str(self.port), "WONTON_TUNNEL_PORT": str(self.udp),
               "WONTON_CORE_CONTAINER": self.core, "WONTON_INSTANCE": "frentry" + self.label,
               "WONTON_RELAYS": f"https://iroh.{self.cloud['zone']}"}
        private(self.install / ".env", "".join(f"{key}={value}\n" for key, value in env.items()))
        compose = json.loads((self.install / "compose.yaml").read_text())
        # 家里的网络不能和云网络的 /24 重叠，否则别名地址落在两个接口的区间里。
        base = ipaddress.ip_network("10.215.0.0/16")
        start = 0 if self.label == "a" else 4
        segments = list(base.subnets(new_prefix=26))[start:start + 4]
        for name, segment in zip(["core", "clients", "edge", "relay"], segments):
            # compose 渲染时没有额外配置的网络是 null，不是空对象。
            compose["networks"][name] = compose["networks"].get(name) or {}
            compose["networks"][name]["ipam"] = {"config": [{"subnet": str(segment)}]}
        compose["networks"]["cloud"] = {"external": True, "name": self.cloud["network"]}
        # 核心在云网络上取一个固定地址：隧道绑在它上面，也把它宣告给对端，解析器不参与。
        address = str(ipaddress.ip_network(self.cloud["subnet"]).network_address + (30 if self.label == "a" else 31))
        for service in ["core", "connector", "tuwunel"]:
            # C5 起入口和连接器由核心按设置建，旧核心的 compose 里还有 connector：没有就跳过。
            spec = compose["services"].get(service)
            if spec is None:
                continue
            # 先测试解析器（联邦名字），再 Docker 的 127.0.0.11（内部别名）；区外名字是 REFUSED 会回退。
            spec["dns"] = [self.cloud["resolver"], "127.0.0.11"]
            spec["networks"] = spec.get("networks") or {}
            spec["networks"]["cloud"] = {"aliases": [f"smoke-{self.label}.{self.cloud['zone']}"]}
            spec["volumes"].append(f"{ca}:/entry-test-ca.pem:ro")
            spec["environment"] = spec.get("environment") or {}
            spec["environment"]["SSL_CERT_FILE"] = "/entry-test-ca.pem"
        core = compose["services"]["core"]

        core["networks"]["cloud"]["ipv4_address"] = address
        core["user"] = f"{os.getuid()}:{os.getgid()}"
        core["group_add"] = [str(os.stat("/var/run/docker.sock").st_gid)]
        core["environment"].update({"WONTON_TUNNEL_BIND": address,
                                    "WONTON_ANNOUNCE_ADDRESSES": address,
                                    "WONTON_SYSTEM_PLUGINS": "", "WONTON_DEFAULT_PLUGINS": "", "RUST_LOG": "wonton_core=debug,wonton_tunnel=debug,iroh=debug"})
        core["volumes"].append(f"{self.binary}:/usr/local/bin/wonton-core:ro")
        core["ports"] = [f"127.0.0.1:{self.port}:{self.port}", f"127.0.0.1:{self.udp}:{self.udp}/udp"]
        private(self.install / "compose.yaml", json.dumps(compose))
        self.address = address
        self.started = True
        try:
            self.compose("up", "--detach", "--wait", "--wait-timeout", "180")
        except Exception:
            # 核心起不来通常只能从它自己的日志看出原因。
            logs = run("docker", "logs", "--tail", "120", self.core, check=False)
            print(logs.stdout + logs.stderr, flush=True)
            state = run("docker", "inspect", "-f", "{{json .State}}", self.core, check=False)
            print(state.stdout + state.stderr, flush=True)
            compose_logs = self.compose("logs", "--no-color", "--tail", "120", check=False)
            print(compose_logs.stdout + compose_logs.stderr, flush=True)
            raise
        self.attach_connector()
        self.token = (self.data / "core/owner-token").read_text().strip()
        assert not json.loads((self.data / "core/peers.json").read_text()).get("peers", [])

    def component_container(self, component):
        """核心经 Docker API 建的组件容器（C5 起入口和连接器不在 compose 里），按组件和实例标签找。"""
        found = run("docker", "ps", "-aq", "--filter", f"label=wonton.core.component={component}",
                    "--filter", f"label=wonton.instance=frentry{self.label}", check=False)
        ids = found.stdout.split()
        return ids[0] if ids else None

    def attach_connector(self):
        """连接器由核心建、只接 edge 和 relay：云栈的测试网络和解析器在 compose 里没有机会注入。

        顺序：接网络 → 重启（frpc 设了 `loginFailExit`，起不来时已经退出，不重启不会重试）→ 再写解析器。
        解析器放最后：`restart` 会按 Daemon 的配置把 `/etc/resolv.conf` 重写掉，先写会被抹掉；写它是 root 的文件
        （容器里跑的是证书属主，非 root），所以要 `-u 0`。
        """
        connector = wait_for("connector container",
                             lambda: self.component_container("connector"), seconds=120)
        run("docker", "network", "connect", self.cloud["network"], connector)
        run("docker", "restart", connector)
        run("docker", "exec", "-u", "0", connector, "sh", "-c",
            f"printf 'nameserver {self.cloud['resolver']}\n' >> /etc/resolv.conf")

    def api(self, path, body=None):
        headers = {"Authorization": "Bearer " + self.token, "Host": f"wonton.localhost:{self.port}"}
        request = Request(f"http://127.0.0.1:{self.port}{path}",
                          data=None if body is None else json.dumps(body).encode(), headers=headers)
        request.add_header("Content-Type", "application/json")
        with build_opener(ProxyHandler({})).open(request, timeout=60) as response:
            return json.load(response)

    def matrix(self, path, body=None, token=None, method=None):
        headers = {"Authorization": "Bearer " + token} if token else {}
        status, result = curl(self.project + "_core", "http://tuwunel.wonton.internal:8008" + path,
                              method or ("POST" if body is not None else "GET"), body, headers)
        assert status in (200, 201), f"Matrix {path}: {status} {result}"
        return json.loads(result)

    def phone_request(self, path, body=None):
        status, result = curl("container:" + self.core, f"http://wonton.localhost:{self.port}" + path,
                              "POST" if body is not None else "GET", body, proxy=self.proxy)
        assert status == 200, f"Device {path}: {status} {result}"
        return json.loads(result)

    def start_phone(self):
        image = os.environ.get("WONTON_TUNNEL_CLIENT_IMAGE", "wonton-tunnel-client:dev")
        args = ["docker", "run", "--rm", "--network", "none", "--user", f"{os.getuid()}:{os.getgid()}",
                "-v", f"{self.dir}:/case", image, "--key", "/case/phone.key", "id"]
        endpoint = run(*args).stdout.strip()
        assert self.api("/_wonton/api/devices", {"endpoint_id": endpoint})["added"]
        server = self.api("/_wonton/api/tunnel")["endpoint_id"]
        run("docker", "run", "--detach", "--name", self.phone, "--network", "container:" + self.core,
            "--user", f"{os.getuid()}:{os.getgid()}", "-v", f"{self.dir}:/case", image,
            "--key", "/case/phone.key", "proxy", "--auth", "--server", server,
            "--addr", f"{self.address}:{self.udp}", "--domain", "wonton.localhost", "--listen", "127.0.0.1:18008")
        def ready():
            logs = run("docker", "logs", self.phone).stdout
            match = re.search(r"会话凭据：([^\s]+)", logs)
            return match.group(1) if match and "本地代理" in logs else None
        self.proxy = wait_for("phone proxy", ready)
        assert self.phone_request("/_wonton/api/peers")["peers"] == []

    def close(self):
        if "--keep" in sys.argv[1:]:
            return
        run("docker", "rm", "--force", self.phone, check=False)
        if self.started:
            # 入口和连接器是核心建的、不带 compose 标签：先删，否则它们挂着的网络 compose 删不掉。
            for component in ("entry", "connector"):
                container = self.component_container(component)
                if container:
                    run("docker", "rm", "--force", container, check=False)
            self.compose("down", "--volumes", "--remove-orphans")
        if self.acquired:
            run(SLOT, "release", self.project)
