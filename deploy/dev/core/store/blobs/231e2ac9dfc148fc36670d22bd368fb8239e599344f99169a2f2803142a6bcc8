"""Wonton 给 Hermes 的两个钩子。

模型网关没接上（注入的模型变量为空）、或网关里还没有默认模型时（主人还没在「模型设置」里添加模型服务商），
直接回一句该怎么做，不调模型。

不拦的话，网关的报错会原样发进聊天：magpie 说没有这个模型的报错很长，绕过了 Hermes 对模型报错的改写。

每条消息先问一次网关有哪些模型（同一个 Docker 网络里，几毫秒）：主人加了 key，下一条消息就照常回答，不用重启。
网关连不上、回答不对时不拦，交给 Hermes 照常处理和报错，这里不猜原因。

每轮带上发文件的办法：Hermes 把工具结果里的预览存成缓存文件，写上 MEDIA:<路径>，它的 Matrix 提示
又教模型用 MEDIA: 发文件，没有这句话时模型很可能把预览发给主人。存文件的办法也在这里：outbox 的路径是 Hermes 的挂载决定的。
同一轮还带一句引导语：主人想要新页面或应用时，先调 wonton__plugin_guide。
工具错误和确认过程也要按结果回答：调用前的预告不是执行结果，返回速度和成功结果都不能证明没有经过确认卡片。
"""

import logging
import os

import httpx

logger = logging.getLogger(__name__)

NO_MODELS = (
    "还没有可用的模型：请在「模型设置」里添加一个模型服务商和它的 API key，再给我发消息。\n"
    "No model is available yet: add a model provider and its API key in Models, then message me again."
)
NO_DEFAULT = (
    "默认模型 {model} 里没有可用的模型：请在「模型设置」里检查名为 {group} 的路由组。\n"
    "The default model {model} has no usable model: check the routing group {group} in Models."
)

SEND_FILES = (
    "发给主人的文件来自工具结果里的编号时（「文件 …，编号 …」）：调 wonton__share_file，把返回的 wonton://file/ 链接原样放进回复，"
    "不要改动它，也不要用 MEDIA: 发这个文件或它的预览：预览只是缩小的图。你自己生成的图片和语音照旧用 MEDIA:。"
    "主人要把文件放进音乐库这类共享文件夹时：把文件放进 /opt/data/outbox，用 wonton__list_shares 看有哪些文件夹，再调 wonton__save_file，"
    "这是唯一的办法。"
)

WRITE_PLUGIN = "主人想要一个新的页面、应用或小工具，或者要改你之前写的插件时：先调 wonton__plugin_guide，照它说的做。"

TOOL_RESULTS = (
    "只按实际工具结果汇报，没调用的工具不能说已查过或执行过。调用前只预告将请求操作，不能说已开始、已完成或确认卡片已发出。"
    "Hermes 将工具失败标为顶层非空 error：遇到它，说明哪一步没成、结果给出的原因（用人话）和用户能做什么；不能继续说已开始或已完成。"
    "结果不确定就说不确定。确认卡片由核心处理，返回快或返回成功都不能证明没经过确认；"
    "结果没明确说明本次是否经过主人确认时，说不知道，不能编造没发卡片、绕过确认或主人已经点了允许。"
)


async def missing_model_hint(text="", **_):
    """``post_gateway_admission``：默认模型不在网关的模型列表里时，回复提示，这条消息不再交给模型。"""
    if text.lstrip().startswith("/"):
        return None
    base = os.environ.get("WONTON_MODEL_BASE_URL", "").rstrip("/")
    key = os.environ.get("WONTON_MODEL_API_KEY", "")
    model = os.environ.get("WONTON_MODEL_DEFAULT", "")
    if not (base and key and model):
        # 模型网关没接上（变量为空，spec：“models”是“有网关就用”）：主人的模型还配在网关这个 provider 上时，
        # 直接给指引，不把 Hermes 对空地址的原始报错发进聊天；主人已经换成自己的服务商就不拦。
        if _gateway_provider_active():
            return {"action": "handled", "reply": NO_MODELS}
        return None
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            response = await client.get(f"{base}/models", headers={"Authorization": f"Bearer {key}"})
            response.raise_for_status()
            ids = {item["id"] for item in response.json()["data"]}
    except (httpx.HTTPError, ValueError, KeyError, TypeError) as e:
        logger.warning("查询模型网关的模型列表失败，照常交给 Hermes：%r", e)
        return None
    if model in ids:
        return None
    if ids:
        reply = NO_DEFAULT.format(model=model, group=model.removeprefix("group/"))
    else:
        reply = NO_MODELS
    return {"action": "handled", "reply": reply}


def _gateway_provider_active() -> bool:
    """主人的模型还配在网关这个 provider 上才算（配置读不出来时不拦，不误导）。"""
    try:
        from hermes_cli.config import load_config_readonly

        model_cfg = (load_config_readonly() or {}).get("model") or {}
        provider = str(model_cfg.get("provider") or "").strip()
    except Exception:
        logger.warning("读 config.yaml 里的模型 provider 失败，不当成网关处理", exc_info=True)
        return False
    return provider in ("", "magpie", "custom:magpie")


def send_files_context(**_):
    """``pre_llm_call``：返回的文字追加在这一轮的用户消息后面，不进系统提示。"""
    return {"context": f"{SEND_FILES}\n{WRITE_PLUGIN}\n{TOOL_RESULTS}"}


def register(ctx):
    ctx.register_hook("post_gateway_admission", missing_model_hint)
    ctx.register_hook("pre_llm_call", send_files_context)
