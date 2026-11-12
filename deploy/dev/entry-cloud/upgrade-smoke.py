#!/usr/bin/env python3
"""#88 的升级实测：新布局的挂载/权限，以及从 main 旧布局升级后状态是否还在。

跑法：HEAD_BINARY=<装好后布局的核心 ELF> MAIN_BINARY=<旧布局（无入口）的核心 ELF>
  python3 deploy/dev/entry-cloud/upgrade-smoke.py
ELF 要在和镜像同一 glibc 的容器里编（宿主直接编的可能跑不进镜像）。脚本自己拿名额、起栈、拆栈。
"""
import hashlib
import hmac
import ipaddress
import json
import os
import shutil
import socket
import subprocess
import sys
import time
import tempfile
import yaml
from pathlib import Path

HERE = Path(__file__).resolve().parent
CLOUD = HERE
sys.path.insert(0, str(CLOUD))
from smoke_support import SLOT, Home, curl, free_port, run, wait_for  # noqa: E402

HEAD = Path(os.environ["HEAD_BINARY"]).resolve()
MAIN = Path(os.environ["MAIN_BINARY"]).resolve()
HOME_BINARY = Path(os.environ["WONTON_HOME_BINARY"]).resolve() if os.environ.get("WONTON_HOME_BINARY") else None
CORE_IMAGE = os.environ.get("WONTON_ENTRY_CORE_IMAGE", "wonton-core:e2e-main")
RUNTIME_IMAGE = os.environ.get(
    "WONTON_HOME_RUNTIME_IMAGE",
    "public.ecr.aws/docker/library/debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4",
)


def entry(home, path):
    status, body = curl(home.cloud["network"], "https://" + home.name + path, ca=home.cloud["ca"],
                        dns=home.cloud["resolver"])
    assert status == 200, f"{path}: {status} {body}"
    return json.loads(body)


def ready_entry(home):
    def probe():
        try:
            status, body = curl(home.cloud["network"], "https://" + home.name + "/.well-known/matrix/server",
                                ca=home.cloud["ca"], dns=home.cloud["resolver"])
        except ConnectionError:
            return False
        return status == 200 and json.loads(body) == {"m.server": home.name + ":443"}
    wait_for("entry " + home.label, probe, seconds=420)


def login(home):
    """主人的设备经隧道要一次性登录令牌，再拿它登录 Matrix：核心签的 JWT 得被 Tuwunel 认。"""
    token = home.phone_request("/_wonton/matrix/login-token", {})
    session = home.matrix("/_matrix/client/v3/login",
                          {"type": token["login_type"], "token": token["token"], "initial_device_display_name": "smoke"})
    assert session["user_id"] == "@owner:" + home.name, session
    return session["access_token"]


def tuwenel_container(home):
    out = run("docker", "ps", "--quiet", "--filter", "label=com.docker.compose.project=" + home.project,
              "--filter", "label=com.docker.compose.service=tuwunel").stdout.split()
    assert len(out) == 1, out
    return out[0]


def check_mounts(home):
    container = tuwenel_container(home)
    mounts = json.loads(run("docker", "inspect", container, "--format", "{{json .Mounts}}").stdout)
    # 测试自己给每个服务挂的 CA，不算安装布局。
    mounts = [mount for mount in mounts if mount["Destination"] != "/entry-test-ca.pem"]
    sources = [mount["Source"] for mount in mounts]
    expected = str(home.install / "secrets/tuwunel")
    by_destination = {mount["Destination"]: mount for mount in mounts}
    assert set(by_destination) == {"/var/lib/tuwunel", "/etc/tuwunel"}, sources
    assert by_destination["/var/lib/tuwunel"]["Name"] == home.project + "_tuwunel", sources
    bind = by_destination["/etc/tuwunel"]
    assert bind["Source"] == expected and not bind["RW"], bind
    # 镜像里没有 shell（distroless），看挂载源目录就等于看容器看到的内容。
    listing = [entry.name for entry in (home.install / "secrets/tuwunel").iterdir()]
    assert "tuwunel.toml" in listing and "wonton-fed.toml" in listing, listing
    for leaked in ["matrix-jwt.pem", "matrix-registration-secret", "hydra-secret", "hydra.env"]:
        assert leaked not in listing, listing
    state = run("docker", "inspect", container, "--format",
                "{{.State.Health.Status}}/{{.State.Status}}").stdout.strip()
    assert state.startswith("healthy"), state
    print(f"PASS mounts and contents ({home.label})", flush=True)


def fetched(home, path):
    """连接器经中继的时刻会换，取不到就当还没好。"""
    try:
        return entry(home, path)
    except (ConnectionError, AssertionError):
        return None


def check_entry(home):
    ready_entry(home)
    try:
        wait_for("well-known server", lambda: fetched(home, "/.well-known/matrix/server")
                 == {"m.server": home.name + ":443"}, seconds=200)
        wait_for("well-known client", lambda: fetched(home, "/.well-known/matrix/client")
                 == {"m.homeserver": {"base_url": "https://" + home.name}}, seconds=200)
        keys = wait_for("federation keys", lambda: fetched(home, "/_matrix/key/v2/server"), seconds=200)
    except RuntimeError:
        for component in ("entry", "connector"):
            container = home.component_container(component)
            if container:
                print(run("docker", "logs", "--tail", "30", container, check=False).stdout, flush=True)
        print(run("docker", "logs", "--tail", "30", home.core, check=False).stdout, flush=True)
        raise
    assert keys["server_name"] == home.name
    print(f"PASS discovery and signing keys ({home.label})", flush=True)


def account(home, username):
    nonce = home.matrix("/_synapse/admin/v1/register")["nonce"]
    password = os.urandom(32).hex()
    secret = (home.install / "secrets/matrix-registration-secret").read_text().strip().encode()
    mac = hmac.new(secret, "\0".join([nonce, username, password, "notadmin"]).encode(), hashlib.sha1).hexdigest()
    return home.matrix("/_synapse/admin/v1/register", {
        "nonce": nonce, "username": username, "password": password, "mac": mac,
    })


def install(home, acquire, entry=True):
    """Home.install_home 的搬运版：升级时不再拿第二个名额。"""
    if acquire:
        # 名额常被占满；家里两套栈只多占一条云网络地址，没有空名额就不登记直接起（和 smoke.py 一样）。
        free = int(run(SLOT, "free").stdout.strip() or 0)
        if free > 0:
            subprocess.run([str(SLOT), "acquire", "--project", home.project, home.project], check=True)
            home.acquired = True
        else:
            print(f"{home.project}: 没有空名额，不登记直接起", flush=True)
    ca = home.cloud["ca"]
    inputs = home.install / "entry-input"
    inputs.mkdir(mode=0o700, exist_ok=True)
    (inputs / "invite").write_text(run("bash", CLOUD / "dev.sh", "invite", "3600").stdout.strip())
    shutil.copyfile(ca, inputs / "ca.pem")
    values = {
        "WONTON_DATA_DIR": str(home.data), "WONTON_CLIENT_SERVICE": f"https://names.{home.cloud['zone']}:8443",
        "WONTON_CLIENT_INVITE_FILE": str(inputs / "invite"), "WONTON_CLIENT_CA_FILE": str(inputs / "ca.pem"),
        "WONTON_FRP_CA_FILE": str(inputs / "ca.pem"), "WONTON_ACME_CA_FILE": str(inputs / "ca.pem"),
        "WONTON_ACME_DIRECTORY": f"https://pebble.{home.cloud['zone']}:14000/dir",
        "WONTON_ACME_ACCEPT_TERMS": "true", "WONTON_ACME_NETWORK": home.cloud["network"],
        "WONTON_ACME_RESOLVERS": home.cloud["resolver"] + ":53",
        "WONTON_RELAYS": f"https://iroh.{home.cloud['zone']}",
    }
    if not entry:
        # 无入口的那台用私用名，旧版本也认这个变量。
        values["WONTON_MATRIX_NAME_MODE"] = "none"
    (inputs / "init.env").write_text("".join(f"{key}={value}\n" for key, value in values.items()))
    def init_once():
        return run("docker", "run", "--rm", "--network", home.cloud["network"], "--dns", home.cloud["resolver"],
                   "--user", f"{os.getuid()}:{os.getgid()}", "--group-add", str(os.stat("/var/run/docker.sock").st_gid),
                   "--env-file", inputs / "init.env", "-v", "/var/run/docker.sock:/var/run/docker.sock",
                   "-v", f"{home.dir}:{home.dir}", "-v", f"{home.binary}:/usr/local/bin/wonton-core:ro",
                   CORE_IMAGE, "init", home.install, "wonton.localhost", timeout=900, check=False)

    for attempt in (1, 2, 3):
        result = init_once()
        home.name = (home.install / "secrets/matrix-server-name").read_text().strip()
        ok = (home.install / "entry/settings.json").exists() and home.name.endswith(home.cloud["zone"])
        if not entry:
            ok = home.name.endswith(".wonton.internal") and not (home.install / "entry").exists()
        print(f"init {home.label} #{attempt}: exit {result.returncode} name {home.name} entry {ok}", flush=True)
        if ok:
            break
        print(result.stdout + result.stderr, flush=True)
        assert attempt < 3, f"{home.label} 装不出带入口的配置"
        (inputs / "invite").write_text(run("bash", CLOUD / "dev.sh", "invite", "3600").stdout.strip())
    env = {"WONTON_CORE_IMAGE": CORE_IMAGE, "WONTON_INSTALL_DIR": str(home.install),
           "WONTON_DATA_DIR": str(home.data), "WONTON_BASE_DOMAIN": "wonton.localhost",
           "WONTON_PORT": str(home.port), "WONTON_TUNNEL_PORT": str(home.udp),
           "WONTON_CORE_CONTAINER": home.core, "WONTON_INSTANCE": "frentry" + home.label,
           "WONTON_RELAYS": f"https://iroh.{home.cloud['zone']}"}
    if not entry:
        # 私用名的连接器会发布一个宿主端口，避开云栈的 8443。
        env["WONTON_ENTRY_PORT"] = str(free_port())
    (home.install / ".env").write_text("".join(f"{key}={value}\n" for key, value in env.items()))
    compose_path = home.install / "compose.yaml"
    text = ""
    for _ in range(240):
        if compose_path.exists():
            text = compose_path.read_text()
            if text.strip():
                break
        time.sleep(0.25)
    if not text.strip():
        print(f"{compose_path} 是空的：{list(home.install.iterdir())}", flush=True)
    # main 写的还是原始 YAML（渲染成 JSON 是 head 才有的），升级路径要能读两种。
    try:
        compose = json.loads(text)
    except ValueError:
        compose = yaml.safe_load(text)
    base = ipaddress.ip_network("10.215.0.0/16")
    start = 0 if home.label == "a" else 4
    segments = list(base.subnets(new_prefix=26))[start:start + 4]
    for name, segment in zip(["core", "clients", "edge", "relay"], segments):
        compose["networks"][name] = compose["networks"].get(name) or {}
        compose["networks"][name]["ipam"] = {"config": [{"subnet": str(segment)}]}
    compose["networks"]["cloud"] = {"external": True, "name": home.cloud["network"]}
    address = str(ipaddress.ip_network(home.cloud["subnet"]).network_address + (30 if home.label == "a" else 31))
    for service in ["core", "connector", "tuwunel"]:
        if service not in compose["services"]:
            continue
        spec = compose["services"][service]
        spec["dns"] = [home.cloud["resolver"], "127.0.0.11"]
        spec["networks"] = spec.get("networks") or {}
        spec["networks"]["cloud"] = {"aliases": [f"smoke-{home.label}.{home.cloud['zone']}"]}
        spec["volumes"].append(f"{ca}:/entry-test-ca.pem:ro")
        spec["environment"] = spec.get("environment") or {}
        spec["environment"]["SSL_CERT_FILE"] = "/entry-test-ca.pem"
    core = compose["services"]["core"]
    core["networks"]["cloud"]["ipv4_address"] = address
    core["user"] = f"{os.getuid()}:{os.getgid()}"
    core["group_add"] = [str(os.stat("/var/run/docker.sock").st_gid)]
    core["environment"].update({"WONTON_TUNNEL_BIND": address, "WONTON_ANNOUNCE_ADDRESSES": address,
                                "WONTON_SYSTEM_PLUGINS": "", "WONTON_DEFAULT_PLUGINS": ""})
    core["volumes"].append(f"{home.binary}:/usr/local/bin/wonton-core:ro")
    core["ports"] = [f"127.0.0.1:{home.port}:{home.port}", f"127.0.0.1:{home.udp}:{home.udp}/udp"]
    (home.install / "compose.yaml").write_text(json.dumps(compose))
    home.address = address
    home.started = True
    up = home.compose("up", "--detach", "--wait", "--wait-timeout", "420", check=False)
    if up.returncode:
        for service in ("tuwunel", "core"):
            logs = run("docker", "logs", "--tail", "40", home.project + "-" + service, check=False)
            print(f"--- {service}\n" + logs.stdout + logs.stderr, flush=True)
        raise RuntimeError(f"{home.label} 起不来：" + up.stdout + up.stderr)
    if entry:
        home.attach_connector()
    home.token = (home.data / "core/owner-token").read_text().strip()
    print(f"Installed {home.label}: {home.name}", flush=True)


def start_phone(home):
    """和 Home.start_phone 一样，但设备可能已经登记过（升级后重连）。"""
    image = os.environ.get("WONTON_TUNNEL_CLIENT_IMAGE", "wonton-tunnel-client:dev")
    run("docker", "rm", "--force", home.phone, check=False)
    args = ["docker", "run", "--rm", "--network", "none", "--user", f"{os.getuid()}:{os.getgid()}",
            "-v", f"{home.dir}:/case", image, "--key", "/case/phone.key", "id"]
    endpoint = run(*args).stdout.strip()
    home.api("/_wonton/api/devices", {"endpoint_id": endpoint})
    server = home.api("/_wonton/api/tunnel")["endpoint_id"]
    run("docker", "run", "--detach", "--name", home.phone, "--network", "container:" + home.core,
        "--user", f"{os.getuid()}:{os.getgid()}", "-v", f"{home.dir}:/case", image,
        "--key", "/case/phone.key", "proxy", "--auth", "--server", server,
        "--addr", f"{home.address}:{home.udp}", "--domain", "wonton.localhost", "--listen", "127.0.0.1:18008")
    home.proxy = wait_for("phone", lambda: _phone_ready(home), seconds=120)


def main():
    root = Path(tempfile.mkdtemp(prefix="wonton-fr88-"))
    print("Artifacts: " + str(root), flush=True)
    env = dict(os.environ, COMPOSE_PROJECT_NAME="wonton-fr-entry", WONTON_HOME_ZONE="homes.test")
    if HOME_BINARY:
        env["WONTON_HOME_BINARY"] = str(HOME_BINARY)
        env["WONTON_HOME_RUNTIME_IMAGE"] = RUNTIME_IMAGE
    for key in ["API", "RELAY", "DNS", "AUTH", "PEBBLE", "PEBBLE_MANAGEMENT"]:
        env["WONTON_ENTRY_" + key + "_PORT"] = str(free_port())
    acquired, cloud_up, homes = False, False, []
    try:
        subprocess.run([str(SLOT), "acquire", "--project", "wonton-fr-entry", "fr-entry"], check=True)
        acquired = True
        env["WONTON_DEV_CORE_SUBNET"] = run(SLOT, "subnet", "fr-entry").stdout.strip()
        cloud_up = True
        run("bash", CLOUD / "dev.sh", "up", env=env, timeout=900)
        values = dict(line.split("=", 1) for line in (CLOUD / ".runtime/stack.env").read_text().splitlines())
        cloud = {"zone": values["WONTON_HOME_ZONE"], "network": values["WONTON_ENTRY_NETWORK"],
                 "resolver": values["WONTON_ENTRY_RESOLVER_IP"], "relay": values["WONTON_ENTRY_RELAY_IP"],
                 "relay_port": values["WONTON_ENTRY_RELAY_PORT"], "subnet": values["WONTON_DEV_CORE_SUBNET"],
                 "ca": (CLOUD / ".runtime/ca/home-trust.pem").resolve()}

        # a) 全新安装（head）
        a = Home("a", root, cloud, HEAD, CORE_IMAGE)
        homes.append(a)
        install(a, acquire=True)
        a.start_phone()
        check_mounts(a)
        assert login(a)
        print("PASS owner login with a core-signed token (a)", flush=True)
        check_entry(a)

        # b) main 旧布局装上、配对、发消息，再换 head 升级
        b = Home("b", root, cloud, MAIN, CORE_IMAGE)
        homes.append(b)
        install(b, acquire=True, entry=False)
        b.start_phone()
        name_before = b.name
        alice = account(b, "alice")
        room = b.matrix("/_matrix/client/v3/createRoom", {"name": "upgrade"}, alice["access_token"])["room_id"]
        body = "升级前的一条 " + os.urandom(4).hex()
        b.matrix(f"/_matrix/client/v3/rooms/{room}/send/m.room.message/{os.urandom(8).hex()}",
                 {"msgtype": "m.text", "body": body}, alice["access_token"], "PUT")
        old_mounts = json.loads(run("docker", "inspect", tuwenel_container(b),
                                    "--format", "{{json .Mounts}}").stdout)
        old_sources = {mount["Destination"]: mount["Source"] for mount in old_mounts}
        assert old_sources["/etc/tuwunel/tuwunel.toml"].endswith("secrets/tuwunel.toml"), old_mounts
        assert "fed/config.toml" in old_sources["/etc/tuwunel/fed.toml"], old_mounts
        assert (b.install / "secrets/tuwunel.toml").exists()
        assert (b.install / "fed/config.toml").exists()
        print("PASS main lays out the old mount", flush=True)
        run("docker", "rm", "--force", b.phone, check=False)
        b.compose("down")

        # 同一个 compose 项目、同一套目录和端口，只把核心二进制换成 head。
        upgraded = Home.__new__(Home)
        upgraded.label, upgraded.cloud, upgraded.project = "b", cloud, b.project
        upgraded.dir, upgraded.install, upgraded.data = b.dir, b.install, b.data
        upgraded.port, upgraded.udp = b.port, b.udp
        upgraded.core, upgraded.phone, upgraded.proxy = b.core, b.phone, b.proxy
        upgraded.acquired, upgraded.started = False, False
        upgraded.binary, upgraded.core_image = HEAD, CORE_IMAGE
        homes.append(upgraded)
        install(upgraded, acquire=False, entry=False)
        assert upgraded.name == name_before, (upgraded.name, name_before)
        start_phone(upgraded)
        login(upgraded)
        print("PASS owner login after upgrade", flush=True)
        messages = upgraded.matrix(
            f"/_matrix/client/v3/rooms/{room}/messages?dir=b&limit=20", token=alice["access_token"])
        assert any(event["content"].get("body") == body for event in messages["chunk"]), messages
        assert not (upgraded.install / "secrets/tuwunel.toml").exists()
        assert not (upgraded.install / "fed/config.toml").exists()
        assert (upgraded.install / "secrets/tuwunel/tuwunel.toml").exists()
        assert (upgraded.install / "secrets/tuwunel/tuwunel-tls.crt").exists(), "证书要跟着搬"
        assert (upgraded.install / "secrets/tuwenel-tls.crt").exists() is False
        print("PASS name, account, message and file move survive the upgrade", flush=True)
        check_mounts(upgraded)
        print("PASS #88 upgrade acceptance", flush=True)
    finally:
        for home in reversed(homes):
            home.close()
        keep = "--keep" in sys.argv[1:] or os.environ.get("KEEP")
        if cloud_up and not keep:
            print(run("bash", CLOUD / "dev.sh", "destroy", env=env).stdout, flush=True)
        if acquired:
            run(SLOT, "release", "fr-entry")
        if keep:
            print("Kept " + str(root), flush=True)
        else:
            shutil.rmtree(root)
            print("Cleaned", flush=True)


def _phone_ready(home):
    import re
    logs = run("docker", "logs", home.phone, check=False).stdout
    match = re.search(r"会话凭据：(\S+)", logs)
    return match.group(1) if match and "本地代理" in logs else None


if __name__ == "__main__":
    main()
