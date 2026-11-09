/**
 * WebSocket 终端协议常量。
 *
 * ⚠️ 与 backend/src/terminal/ws.rs 顶部的常量保持同步（改动必须双端同时）。
 */

/** 二进制帧类型：浏览器 → Shell 输入。 */
export const INPUT_TYPE = 0x01;
/** 二进制帧类型：Shell → 浏览器输出。 */
export const OUTPUT_TYPE = 0x02;
/** 输出帧头部长度：1 字节类型 + 8 字节大端序列号（u64 BE）。 */
export const OUTPUT_HEADER_LEN = 9;
/** 自定义关闭码：连接被其他窗口接管。 */
export const CLOSE_REPLACED = 4001;
/** 心跳间隔：必须小于后端 TERMINAL_PONG_TIMEOUT（60s）。 */
export const PING_INTERVAL_MS = 30_000;
/** 看门狗：超过该时间未收到服务端任何消息则主动断开重连（与后端判死值一致）。 */
export const PONG_TIMEOUT_MS = 60_000;
