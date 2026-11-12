#!/usr/bin/env python3
"""打印当前 Android 界面上的可读节点：bounds、clickable、text/content-desc。"""
import re, subprocess, sys, xml.etree.ElementTree as ET
ADB = "/home/npchy/android-sdk/platform-tools/adb"
S = ["-s", "emulator-5582"]
subprocess.run([ADB] + S + ["shell", "uiautomator", "dump", "/sdcard/ui.xml"], capture_output=True)
raw = subprocess.run([ADB] + S + ["shell", "cat", "/sdcard/ui.xml"], capture_output=True, text=True).stdout
root = ET.fromstring(raw[raw.index("<hierarchy"):])
def walk(n):
    lab = n.get("text") or n.get("content-desc") or ""
    if lab:
        print(f'{n.get("bounds")}\t{n.get("clickable")}\t{n.get("class","").split(".")[-1]}\t{lab[:90]}')
    for c in n:
        walk(c)
walk(root)
