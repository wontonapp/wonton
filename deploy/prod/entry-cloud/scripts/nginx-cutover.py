#!/usr/bin/env python3
"""Move an existing nginx behind frps without touching what it serves.

Phase 1 (check / apply / rollback): every server block that listens on 80/443
keeps its body; only those listen lines move to 127.0.0.1:4080/4443 with
PROXY protocol, and each moved 443 block gets an ACME webroot location so the
certbot lineages can renew with `--webroot`. Before writing anything, every
name, wildcard and default_server that nginx answers on 80/443 today is
compared with what system-frpc will actually route back to nginx; anything
that would stop working makes the script refuse unless it is named with
--accept. The point is that another site on the same host is never broken
silently.

Phase 2 (swap): replace named sites-enabled entries with rendered templates.

`renewal-check` confirms `certbot reconfigure` switched the lineages to webroot
and kept their original hooks.
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

NGINX_CONF = Path("/etc/nginx/nginx.conf")
SITES = Path("/etc/nginx/sites-enabled")
CONF_D = Path("/etc/nginx/conf.d")
RENEWAL = Path("/etc/letsencrypt/renewal")
WEBROOT = Path("/var/www/letsencrypt")
TARGET = {"http": ("127.0.0.1:4080", 4080), "https": ("127.0.0.1:4443", 4443)}
PORT_KIND = {80: "http", 443: "https"}
KEEP_PARAMS = {"ssl", "http2", "default_server", "default"}
# Socket options that only matter on the public socket; nginx refuses them
# twice on one address, and two old listens can collapse into one loopback.
DROP_PARAMS = re.compile(r"^(ipv6only=(on|off)|reuseport|deferred|bind|backlog=.*|fastopen=.*|"
                         r"so_keepalive=.*|rcvbuf=.*|sndbuf=.*)$")
ACME_LOCATION = "location ^~ /.well-known/acme-challenge/ { root %s; }  # wonton cutover"
MANIFEST = "manifest.json"


class Refuse(Exception):
    pass


@dataclass
class Directive:
    name: str
    args: list
    file: Path
    line: int
    start: int
    end: int
    block: list | None = None
    body_start: int = 0


@dataclass
class Plan:
    edits: dict = field(default_factory=dict)  # Path -> [(start, end, text)]
    problems: list = field(default_factory=list)
    uncovered: list = field(default_factory=list)  # (kind, name, where)
    warnings: list = field(default_factory=list)
    listeners: list = field(default_factory=list)  # (port, where) left on 80/443

    def edit(self, path, start, end, text):
        self.edits.setdefault(path, []).append((start, end, text))


# ---------------------------------------------------------------- parsing


def tokenize(text):
    i, n, line = 0, len(text), 1
    while i < n:
        ch = text[i]
        if ch == "\n":
            line += 1
            i += 1
        elif ch.isspace():
            i += 1
        elif ch == "#":
            while i < n and text[i] != "\n":
                i += 1
        elif ch in "{};":
            yield ch, i, i + 1, line, True
            i += 1
        elif ch in "\"'":
            start, i, value = i, i + 1, []
            while i < n and text[i] != ch:
                if text[i] == "\\" and i + 1 < n:
                    i += 1
                if text[i] == "\n":
                    line += 1
                value.append(text[i])
                i += 1
            yield "".join(value), start, i + 1, line, False
            i += 1
        else:
            start = i
            while i < n and not text[i].isspace() and text[i] not in ";{}":
                if text[i] == "$" and i + 1 < n and text[i + 1] == "{":
                    i = text.index("}", i) + 1
                    continue
                i += 1
            yield text[start:i], start, i, line, False


def parse_file(path, conf_dir, seen=()):
    if path in seen:
        raise Refuse(f"include loop at {path}")
    tokens = list(tokenize(path.read_text()))
    pos = 0

    def block(depth):
        nonlocal pos
        out = []
        while pos < len(tokens):
            tok, start, _, line, delim = tokens[pos]
            if delim and tok == "}":
                if depth == 0:
                    raise Refuse(f"{path}:{line}: unbalanced '}}'")
                pos += 1
                return out
            if delim:
                raise Refuse(f"{path}:{line}: unexpected '{tok}'")
            args = []
            pos += 1
            while pos < len(tokens) and not tokens[pos][4]:
                args.append(tokens[pos][0])
                pos += 1
            if pos >= len(tokens) or tokens[pos][0] == "}":
                raise Refuse(f"{path}:{line}: unterminated directive {tok}")
            end, opener = tokens[pos][2], tokens[pos][0]
            pos += 1
            d = Directive(tok, args, path, line, start, end)
            if opener == "{":
                d.body_start = end
                d.block = block(depth + 1)
            if tok == "include" and args:
                for inc in sorted(glob.glob(str(conf_dir / args[0]))):
                    out.extend(parse_file(Path(inc), conf_dir, seen + (path,)))
            else:
                out.append(d)
        return out

    return block(0)


def listen_port(arg):
    if arg.startswith("unix:"):
        return None
    if arg.isdigit():
        return int(arg)
    if arg.startswith("["):
        tail = arg.split("]", 1)[1]
        return int(tail[1:]) if tail.startswith(":") else 80
    host, _, port = arg.rpartition(":")
    if host and port.isdigit():
        return int(port)
    return 80  # address without port


def servers(tree):
    """Yield (context, server directive) for every http/stream server block."""
    for d in tree:
        if d.name in ("http", "stream") and d.block is not None:
            for s in d.block:
                if s.name == "server" and s.block is not None:
                    yield d.name, s


# ---------------------------------------------------------------- coverage


def routed_domains(frpc_path, allowlist):
    """Domains system-frpc brings back to nginx, by kind, after the hook allowlist."""
    config = tomllib.loads(frpc_path.read_text())
    covered = {"http": set(), "https": set()}
    problems = []
    for proxy in config.get("proxies", []):
        kind = proxy.get("type")
        domains = [d.lower() for d in proxy.get("customDomains", []) if d]
        if kind not in covered:
            continue
        if proxy.get("localPort") != TARGET[kind][1]:
            problems.append(f"system-frpc proxy {proxy.get('name')} ({kind}) points at "
                            f"localPort {proxy.get('localPort')}, nginx will listen on {TARGET[kind][1]}")
            continue
        rejected = [d for d in domains if d not in allowlist]
        if rejected:
            problems.append(f"system-frpc proxy {proxy.get('name')} has {', '.join(rejected)} outside "
                            "WONTON_HOME_SYSTEM_DOMAINS; the hook rejects the whole proxy")
            continue
        covered[kind].update(domains)
    return covered, problems


def plan_server(server, sites, covered, accepted, plan, webroot, defaults):
    where = f"{server.file}:{server.line}"
    listens = [d for d in server.block if d.name == "listen"]
    names = [a.lower() for d in server.block if d.name == "server_name" for a in d.args] or [""]
    if not listens:
        plan.problems.append(f"{where}: server block has no listen, nginx binds *:80 for it")
        return
    moved = {}
    for d in listens:
        port = listen_port(d.args[0]) if d.args else None
        if port in (4080, 4443):
            plan.problems.append(f"{d.file}:{d.line}: already listens on {port}; was cutover applied already?")
            continue
        if port not in PORT_KIND:
            continue
        kind = PORT_KIND[port]
        params = d.args[1:]
        bad = [p for p in params if p not in KEEP_PARAMS and not DROP_PARAMS.match(p)]
        if bad:
            plan.problems.append(f"{d.file}:{d.line}: listen parameter(s) {' '.join(bad)} not handled")
            continue
        if (kind == "https") != ("ssl" in params):
            plan.problems.append(f"{d.file}:{d.line}: port {port} {'without' if kind == 'https' else 'with'} ssl")
            continue
        if d.file.parent != sites:
            plan.problems.append(f"{d.file}:{d.line}: listen {port} lives outside {sites}; move it by hand")
            continue
        for name in names:
            label = name or '""'
            if name not in covered[kind] and f"{kind}:{label}" not in accepted:
                plan.uncovered.append((kind, label, where))
        # Bare-IP and unknown-Host requests land on the default server today;
        # frps answers those itself afterwards.
        if any(p in ("default_server", "default") for p in params) or kind not in defaults:
            defaults[kind] = where
        if kind in moved:
            plan.edit(d.file, d.start, d.end, "# wonton cutover: " + " ".join([d.name] + d.args) + ";")
            continue
        keep = [p for p in params if p in KEEP_PARAMS]
        moved[kind] = d
        plan.edit(d.file, d.start, d.end, " ".join(["listen", TARGET[kind][0], *keep, "proxy_protocol"]) + ";")
    if "https" in moved:
        plan_acme(server, plan, where, webroot)


def plan_acme(server, plan, where, webroot):
    for d in server.block:
        if d.name == "location" and any("acme-challenge" in a for a in d.args):
            roots = [a for x in d.block or [] if x.name in ("root", "alias") for a in x.args]
            if not any(str(webroot) in r for r in roots):
                plan.warnings.append(f"{d.file}:{d.line}: existing acme-challenge location does not use "
                                     f"{webroot}; webroot renewal through this block will fail")
            return
    if any(d.name in ("return", "rewrite") for d in server.block):
        plan.warnings.append(f"{where}: server-level return/rewrite runs before locations; "
                             "no ACME location added, renewal must succeed through the redirect target")
        return
    plan.edit(server.file, server.body_start, server.body_start, "\n    " + ACME_LOCATION % webroot)


def analyse(args):
    allowlist = {d.strip().lower() for d in os.environ.get("WONTON_HOME_SYSTEM_DOMAINS", "").split(",")}
    allowlist.discard("")
    if not allowlist:
        raise Refuse("WONTON_HOME_SYSTEM_DOMAINS is empty; source .env first")
    covered, problems = routed_domains(args.frpc, allowlist)
    plan = Plan(problems=problems)
    accepted = {a.lower() for a in args.accept or []}
    defaults = {}
    tree = parse_file(args.nginx_conf, args.nginx_conf.parent)
    for ctx, server in servers(tree):
        if ctx == "stream":
            for d in server.block:
                if d.name == "listen" and d.args and listen_port(d.args[0]) in PORT_KIND:
                    plan.problems.append(f"{d.file}:{d.line}: stream listener on {d.args[0]} would hold the port")
            continue
        plan_server(server, args.sites, covered, accepted, plan, args.webroot, defaults)
    for kind, where in defaults.items():
        if f"{kind}:default_server" not in accepted:
            plan.uncovered.append((kind, "default_server", f"{where} (bare IP / unknown host)"))
    return plan


def public_listens(args):
    """What would still hold 80/443 (or the implicit *:80) after the edit."""
    left = []
    for _, server in servers(parse_file(args.nginx_conf, args.nginx_conf.parent)):
        listens = [d for d in server.block if d.name == "listen"]
        if not listens:
            left.append(f"{server.file}:{server.line} (no listen)")
        left += [f"{d.file}:{d.line}" for d in listens if d.args and listen_port(d.args[0]) in PORT_KIND]
    return left


def report(plan):
    for w in plan.warnings:
        print(f"warning: {w}", file=sys.stderr)
    ok = True
    if plan.problems:
        ok = False
        print("refusing, nothing was changed:", file=sys.stderr)
        for p in plan.problems:
            print(f"  {p}", file=sys.stderr)
    if plan.uncovered:
        ok = False
        print("refusing: nginx answers these today, but system-frpc will not route them back to nginx:",
              file=sys.stderr)
        for kind, name, where in sorted(set(plan.uncovered)):
            print(f"  {kind:5} {name:32} {where}", file=sys.stderr)
        print("add them to system-frpc and WONTON_HOME_SYSTEM_DOMAINS, or, if losing them is intended, rerun with:",
              file=sys.stderr)
        tokens = sorted({f"{k}:{n}" for k, n, _ in plan.uncovered})
        print("  " + " ".join(f"--accept '{t}'" for t in tokens), file=sys.stderr)
    return ok


# ---------------------------------------------------------------- side effects


def run(*cmd, check=True):
    print("+ " + " ".join(cmd))
    return subprocess.run(cmd, check=check)


def listeners(port):
    out = subprocess.run(["ss", "-lntpH"], check=True, capture_output=True, text=True).stdout
    found = []
    for row in out.splitlines():
        cols = row.split()
        if len(cols) >= 4 and cols[3].rsplit(":", 1)[-1] == str(port):
            procs = re.findall(r'\("([^"]+)"', row)
            found.append((cols[3], procs))
    return found


def wait_ports(want, timeout=15.0):
    """want: {port: predicate(list of (addr, procs)) -> bool}."""
    deadline = time.monotonic() + timeout
    while True:
        state = {port: listeners(port) for port in want}
        if all(pred(state[port]) for port, pred in want.items()):
            return state
        if time.monotonic() > deadline:
            raise Refuse(f"ports did not settle: {state}")
        time.sleep(0.5)


def nginx_holds(entries):
    return any("nginx" in procs for _, procs in entries)


def copy_entry(src, dest):
    if dest.is_symlink() or dest.exists():
        dest.unlink()
    if src.is_symlink():
        os.symlink(os.readlink(src), dest)
    else:
        shutil.copy2(src, dest)


def backup(args, added):
    if args.backup_dir.exists():
        raise Refuse(f"{args.backup_dir} exists; pick a fresh directory")
    args.backup_dir.mkdir(mode=0o700, parents=True)
    (args.backup_dir / "sites").mkdir()
    entries = sorted(p.name for p in args.sites.iterdir())
    for name in entries:
        copy_entry(args.sites / name, args.backup_dir / "sites" / name)
    renewal = args.renewal.is_dir()
    if renewal:
        shutil.copytree(args.renewal, args.backup_dir / "renewal", symlinks=True)
    manifest = {"sites": entries, "added": [str(p) for p in added], "renewal": renewal,
                "sites_dir": str(args.sites), "renewal_dir": str(args.renewal)}
    (args.backup_dir / MANIFEST).write_text(json.dumps(manifest, indent=2) + "\n")


def restore_files(backup_dir, renewal=True):
    manifest = json.loads((backup_dir / MANIFEST).read_text())
    sites = Path(manifest["sites_dir"])
    for added in manifest["added"]:
        Path(added).unlink(missing_ok=True)
    for name in manifest["sites"]:
        copy_entry(backup_dir / "sites" / name, sites / name)
    if renewal and manifest["renewal"]:
        dest = Path(manifest["renewal_dir"])
        for conf in sorted((backup_dir / "renewal").iterdir()):
            copy_entry(conf, dest / conf.name)
        extra = sorted(set(p.name for p in dest.iterdir()) - set(p.name for p in (backup_dir / "renewal").iterdir()))
        if extra:
            print(f"note: left lineages created after the backup: {', '.join(extra)}")
    return manifest


def write_edits(plan):
    for path, edits in plan.edits.items():
        text = path.read_text()
        for start, end, new in sorted(edits, key=lambda e: e[0], reverse=True):
            text = text[:start] + new + text[end:]
        mode = path.stat().st_mode & 0o777
        # Write a real file in sites-enabled; the sites-available original stays as it was.
        path.unlink()
        path.write_text(text)
        path.chmod(mode)


def install_added(args):
    names_dest = args.sites / "wonton-names.conf"
    realip_dest = args.conf_d / "wonton-realip.conf"
    for dest in (names_dest, realip_dest):
        if dest.exists() or dest.is_symlink():
            raise Refuse(f"{dest} already exists")
    return [(args.names_conf, names_dest), (args.realip, realip_dest)]


def cmd_check(args):
    plan = analyse(args)
    if not report(plan):
        return 1
    for path, edits in sorted(plan.edits.items()):
        print(f"{path}: {len(edits)} change(s)")
    print("check ok; apply would only change the lines above")
    return 0


def cmd_apply(args):
    plan = analyse(args)
    if not report(plan):
        return 1
    added = install_added(args)
    backup(args, [dest for _, dest in added])
    print(f"backup at {args.backup_dir}")
    try:
        write_edits(plan)
        for src, dest in added:
            shutil.copyfile(src, dest)
            dest.chmod(0o644)
        args.webroot.mkdir(parents=True, exist_ok=True)
        leftover = public_listens(args)
        if leftover:
            raise Refuse("after edit, still on 80/443: " + ", ".join(leftover))
        run("nginx", "-t")
    except (Refuse, subprocess.CalledProcessError, OSError) as err:
        restore_files(args.backup_dir, renewal=False)
        print(f"apply failed, files restored: {err}", file=sys.stderr)
        return 1
    run("systemctl", "reload", "nginx")
    try:
        wait_ports({80: lambda e: not e, 443: lambda e: not e,
                    4080: nginx_holds, 4443: nginx_holds})
    except Refuse as err:
        print(f"nginx did not move: {err}\nrun: rollback --backup-dir {args.backup_dir} --compose-dir <dir>",
              file=sys.stderr)
        return 1
    print("80/443 are free and nginx is on 127.0.0.1:4080/4443; start frps and system-frpc now")
    return 0


def cmd_rollback(args):
    compose = ["docker", "compose", "--project-directory", str(args.compose_dir),
               "-f", str(args.compose_dir / "compose.yaml"), "--env-file", str(args.compose_dir / ".env")]
    if not args.nginx_only:
        run(*compose, "stop", "frps", "system-frpc")
        wait_ports({80: lambda e: not e, 443: lambda e: not e})
    manifest = restore_files(args.backup_dir, renewal=not args.nginx_only)
    run("nginx", "-t")
    run("systemctl", "reload", "nginx")
    if args.nginx_only:
        print(f"nginx restored from {args.backup_dir}")
        return 0
    wait_ports({80: nginx_holds, 443: nginx_holds})
    print(f"rolled back from {args.backup_dir}: frps/system-frpc stopped, nginx owns 80/443, "
          f"{len(manifest['sites'])} sites and the renewal configs restored")
    return 0


def renewal_params(path):
    values, section = {}, None
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if line.startswith("["):
            section = line
        elif section == "[renewalparams]" and "=" in line:
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip()
    return values


def cmd_renewal_check(args):
    ok = True
    for name in args.cert_name:
        before = renewal_params(args.backup_dir / "renewal" / f"{name}.conf")
        now = renewal_params(args.renewal / f"{name}.conf")
        if now.get("authenticator") != "webroot":
            print(f"{name}: authenticator is {now.get('authenticator')}, want webroot", file=sys.stderr)
            ok = False
        for key in ("renew_hook", "pre_hook", "post_hook"):
            if before.get(key) != now.get(key):
                print(f"{name}: {key} changed: {before.get(key)!r} -> {now.get(key)!r}", file=sys.stderr)
                ok = False
        print(f"{name}: authenticator={now.get('authenticator')} renew_hook={now.get('renew_hook')!r}")
    return 0 if ok else 1


def cmd_swap(args):
    for name in args.remove:
        if not (args.sites / name).exists():
            raise Refuse(f"{args.sites / name} does not exist")
    added = [(Path(src), args.sites / f"wonton-{Path(src).name}") for src in args.install]
    for _, dest in added:
        if dest.exists():
            raise Refuse(f"{dest} already exists")
    backup(args, [dest for _, dest in added])
    try:
        for name in args.remove:
            (args.sites / name).unlink()
        for src, dest in added:
            shutil.copyfile(src, dest)
            dest.chmod(0o644)
        run("nginx", "-t")
    except (Refuse, subprocess.CalledProcessError, OSError) as err:
        restore_files(args.backup_dir, renewal=False)
        print(f"swap failed, files restored: {err}", file=sys.stderr)
        return 1
    run("systemctl", "reload", "nginx")
    print(f"swapped; undo with: rollback --nginx-only --backup-dir {args.backup_dir}")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--nginx-conf", type=Path, default=NGINX_CONF)
    parser.add_argument("--sites", type=Path, default=SITES)
    parser.add_argument("--conf-d", type=Path, default=CONF_D)
    parser.add_argument("--renewal", type=Path, default=RENEWAL)
    parser.add_argument("--webroot", type=Path, default=WEBROOT)
    sub = parser.add_subparsers(dest="cmd", required=True)
    for name in ("check", "apply"):
        p = sub.add_parser(name)
        p.add_argument("--frpc", type=Path, required=True, help="rendered system-frpc.toml")
        p.add_argument("--accept", action="append", metavar="KIND:NAME",
                       help="a name (or default_server) that may stop being served, e.g. 'http:default_server'")
    sub.choices["apply"].add_argument("--names-conf", type=Path, required=True)
    sub.choices["apply"].add_argument("--realip", type=Path, required=True)
    sub.choices["apply"].add_argument("--backup-dir", type=Path, required=True)
    p = sub.add_parser("rollback")
    p.add_argument("--backup-dir", type=Path, required=True)
    p.add_argument("--compose-dir", type=Path, default=Path.cwd())
    p.add_argument("--nginx-only", action="store_true", help="undo a swap: restore sites-enabled only")
    p = sub.add_parser("renewal-check")
    p.add_argument("--backup-dir", type=Path, required=True)
    p.add_argument("--cert-name", action="append", required=True)
    p = sub.add_parser("swap")
    p.add_argument("--backup-dir", type=Path, required=True)
    p.add_argument("--remove", action="append", default=[], help="sites-enabled entry to disable")
    p.add_argument("--install", action="append", default=[], help="rendered vhost to enable")
    args = parser.parse_args()
    if os.geteuid() != 0:
        sys.exit("run as root")
    args.sites, args.renewal = args.sites.resolve(), args.renewal.resolve()
    handler = {"check": cmd_check, "apply": cmd_apply, "rollback": cmd_rollback,
               "renewal-check": cmd_renewal_check, "swap": cmd_swap}[args.cmd]
    try:
        sys.exit(handler(args))
    except Refuse as err:
        sys.exit(f"refusing: {err}")


if __name__ == "__main__":
    main()
