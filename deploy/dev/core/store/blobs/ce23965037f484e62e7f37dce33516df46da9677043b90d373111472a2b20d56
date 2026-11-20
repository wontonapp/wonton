"""在实际开轮时选语言，持久队列的首轮和重放也走同一个官方事件。"""

import importlib.util
from pathlib import Path

# 复用插件按人取语言的实现；从当前 Bot 的数据目录加载，不依赖 Hermes 的私有插件模块名。
_path = Path(__file__).resolve().parents[2] / "plugins/wonton/locale.py"
_spec = importlib.util.spec_from_file_location(f"{__name__}.locale", _path)
_locale = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_locale)


async def handle(event_type, context):
    from agent.secret_scope import current_secret_scope, current_secret_scope_home, set_secret_scope

    user = context.get("user_id")
    sender = user if context.get("platform") == "matrix" and isinstance(user, str) and user.startswith("@") else None
    lang = await _locale.language(sender)
    # t() 读取请求局部的 HERMES_LANGUAGE；不改共享环境、配置文件或原有凭据，未命中的凭据仍按上游规则解析。
    set_secret_scope(
        {**(current_secret_scope() or {}), "HERMES_LANGUAGE": lang},
        profile_home=current_secret_scope_home(),
    )
