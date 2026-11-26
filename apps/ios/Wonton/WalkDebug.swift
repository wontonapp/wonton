import os

/// 走查用：把“旁聊被谁收起、时间线订阅怎么结束”写进系统日志，`simctl spawn … log stream` 能读；只在调试包里看。
let walkLog = Logger(subsystem: "app.wonton.ios", category: "walkthrough")
