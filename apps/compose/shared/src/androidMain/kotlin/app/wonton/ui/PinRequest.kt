package app.wonton.ui

import app.wonton.protocol.TabRef

/** 待固定页面来自核心提供的 tabChoices，不在壳里拼网页地址。 */
data class PinRequest(val tab: TabRef, val title: String)
