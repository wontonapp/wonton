#!/usr/bin/env python3
"""dev.sh friend 用：在插件容器的网络命名空间里抓进来的 HTTP 请求头，打印插件实际收到的样子。

网关到插件是明文 HTTP，在插件这一侧抓包看到的就是插件读到的字节，不用改插件、不用猜网关怎么渲染。
只看进来的包（不看本机回环，那是容器自己的健康检查），只打请求行和头；Cookie、Authorization 只留名字。
"""

import socket
import struct
import sys
import time

METHODS = (b"GET ", b"HEAD ", b"POST ", b"PUT ", b"PATCH ", b"DELETE ", b"OPTIONS ")
SECRET = ("cookie", "authorization")
PACKET_OUTGOING = 4


def show(source, head):
    lines = head.decode("latin-1").split("\r\n")
    print(f"--- {time.strftime('%H:%M:%S')} 来自 {source}", flush=False)
    print(f"  {lines[0]}")
    for line in lines[1:]:
        name, _, value = line.partition(":")
        if name.strip().lower() in SECRET:
            if name.strip().lower() == "cookie":
                value = " " + "; ".join(p.split("=", 1)[0].strip() + "=…" for p in value.split(";"))
            else:
                value = " " + value.strip().split(" ", 1)[0] + " …"
        print(f"  {name}:{value}")
    sys.stdout.flush()


def main():
    raw = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.ntohs(0x0003))
    pending = {}
    while True:
        frame, address = raw.recvfrom(65535)
        interface, protocol, kind = address[0], address[1], address[2]
        if interface == "lo" or kind == PACKET_OUTGOING or protocol != 0x0800:
            continue
        ip = frame[14:]
        if len(ip) < 20 or ip[9] != 6:
            continue
        ihl = (ip[0] & 0x0F) * 4
        total = struct.unpack("!H", ip[2:4])[0]
        tcp = ip[ihl:total]
        if len(tcp) < 20:
            continue
        sport = struct.unpack("!H", tcp[0:2])[0]
        payload = tcp[(tcp[12] >> 4) * 4:]
        if not payload:
            continue
        key = (socket.inet_ntoa(ip[12:16]), sport)
        buffer = pending.get(key, b"")
        if not buffer and not payload.startswith(METHODS):
            continue  # 请求体或别的协议
        buffer += payload
        end = buffer.find(b"\r\n\r\n")
        if end < 0:
            pending[key] = buffer[:65536]
            continue
        pending.pop(key, None)
        show(f"{key[0]}:{key[1]}", buffer[:end])


if __name__ == "__main__":
    main()
