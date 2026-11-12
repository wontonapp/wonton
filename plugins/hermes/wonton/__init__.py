"""Wonton 给 Hermes 的两个钩子。

模型网关里还没有默认模型时（主人还没在「模型设置」里添加模型服务商），直接回一句该怎么做，不调模型。

不拦的话，网关的报错会原样发进聊天：magpie 说没有这个模型的报错很长，绕过了 Hermes 对模型报错的改写。

每条消息先问一次网关有哪些模型（同一个 Docker 网络里，几毫秒）：主人加了 key，下一条消息就照常回答，不用重启。
网关连不上、回答不对时不拦，交给 Hermes 照常处理和报错，这里不猜原因。

每轮带上发文件的办法（ADR-0020 的“Hermes 的 MEDIA:”）：Hermes 把工具结果里的预览存成缓存文件，写上 MEDIA:<路径>，它的 Matrix 提示
又教模型用 MEDIA: 发文件，没有这句话时模型很可能把预览发给主人。存文件的办法（ADR-0027）也在这里：outbox 的路径是 Hermes 的挂载决定的。
同一轮还带一句引导语（ADR-0033）：主人想要新页面或应用时，先调 wonton__plugin_guide。
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


async def missing_model_hint(text="", **_):
    """``post_gateway_admission``：默认模型不在网关的模型列表里时，回复提示，这条消息不再交给模型。"""
    if text.lstrip().startswith("/"):
        return None
    base = os.environ.get("WONTON_MODEL_BASE_URL", "").rstrip("/")
    key = os.environ.get("WONTON_MODEL_API_KEY", "")
    model = os.environ.get("WONTON_MODEL_DEFAULT", "")
    if not (base and key and model):
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


def send_files_context(**_):
    """``pre_llm_call``：返回的文字追加在这一轮的用户消息后面，不进系统提示。"""
    return {"context": f"{SEND_FILES}\n{WRITE_PLUGIN}"}


def register(ctx):
    ctx.register_hook("post_gateway_admission", missing_model_hint)
    ctx.register_hook("pre_llm_call", send_files_context)
