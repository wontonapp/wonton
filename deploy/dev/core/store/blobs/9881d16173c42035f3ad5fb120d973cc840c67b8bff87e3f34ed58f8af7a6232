"""Hermes 控制台的插件后端：把网页连接上的打开者交给 ``wonton.pages``。

Hermes 在控制台启动时 import 这个文件（``dashboard/manifest.json`` 的 ``api``），插件包里就是那时跑起来
的。这里只做入口：找到已经加载的插件包、调用 ``pages.install``；装不上只记日志，网页对话回到只读。

``router`` 是空的：控制台要求这个文件有它，本插件没有自己的接口，网页还是上游那一份。
"""

import importlib
import logging
import sys
from pathlib import Path

from fastapi import APIRouter

logger = logging.getLogger(__name__)

root = Path(__file__).resolve().parents[1] / "__init__.py"


def plugin():
    """已经加载的插件包。Hermes 按插件名放进 ``hermes_plugins``，键可能带后缀，按文件认。"""
    for module in list(sys.modules.values()):
        path = getattr(module, "__file__", None)
        if path and Path(path).resolve() == root:
            return module
    raise ImportError(f"插件包还没加载：{root}")


def turn():
    return importlib.import_module(f"{plugin().__name__}.turn")


router = APIRouter()

try:
    importlib.import_module(f"{plugin().__name__}.pages").install(turn)
except Exception:
    logger.exception("网页控制台接不上，网页对话会按只读")
