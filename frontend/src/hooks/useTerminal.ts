import { useCallback, useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import { ApiError, initTerminal } from "../services/api";
import {
  CLOSE_REPLACED,
  OUTPUT_HEADER_LEN,
  OUTPUT_TYPE,
  PING_INTERVAL_MS,
  PONG_TIMEOUT_MS,
} from "../services/protocol";
import { xtermTheme } from "../theme";

const DEFAULT_FONT_SIZE = 14;
const MIN_FONT_SIZE = 9;
const MAX_FONT_SIZE = 32;
/** 自动重连最大间隔（指数退避 1s/2s/4s/8s/16s/30s）。 */
const MAX_RECONNECT_DELAY_MS = 30_000;
/** 输入过载提示持续时间。 */
const INPUT_OVERFLOW_NOTICE_MS = 3000;

/** 搜索高亮装饰（overview ruler 配色），按次传给 findNext/findPrevious。 */
const SEARCH_DECORATIONS = {
  matchOverviewRuler: "#3f4a5a",
  activeMatchColorOverviewRuler: "#8ab4f8",
};

interface ExitedState {
  code: number | null;
}

export interface TerminalHandle {
  containerRef: RefObject<HTMLDivElement>;
  connected: boolean;
  exited: ExitedState | null;
  /** 连接被其他窗口接管（服务端 close 4001）。 */
  displaced: boolean;
  /** 输入过载（PTY 输入通道满，部分按键被丢弃）。 */
  inputOverloaded: boolean;
  searchVisible: boolean;
  search: (query: string, direction: 1 | -1) => void;
  closeSearch: () => void;
  /** Shell 退出或被接管后重新初始化终端。 */
  restart: () => void;
}

/**
 * xterm.js + WebSocket 生命周期管理：
 * 登录态下初始化终端、断线指数退避自动重连、按序列号恢复输出、
 * 尺寸自适应、复制/粘贴/搜索/字体快捷键。
 */
export function useTerminal(onAuthExpired: () => void): TerminalHandle {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  /** 浏览器已处理的输出序列号（断线恢复位置）。 */
  const lastSequenceRef = useRef(0);
  const terminalIdRef = useRef<string | null>(null);
  const disposedRef = useRef(false);
  /** Shell 已退出：停止自动重连，等待用户重启。 */
  const stoppedRef = useRef(false);
  /** 连接代号：每次 openSocket 递增，用于识别过期连接的异步回调。 */
  const connGenRef = useRef(0);
  /** connect 进行中标志：防止 restart 双击并发建立两条连接。 */
  const connectInFlightRef = useRef(false);
  const reconnectAttemptRef = useRef(0);
  const reconnectTimerRef = useRef<number | null>(null);
  const lastServerMessageRef = useRef(Date.now());
  const fontSizeRef = useRef(DEFAULT_FONT_SIZE);
  const fitPendingRef = useRef(false);
  const overflowTimerRef = useRef<number | null>(null);
  const connectRef = useRef<() => void>(() => {});
  const onAuthExpiredRef = useRef(onAuthExpired);
  onAuthExpiredRef.current = onAuthExpired;

  const [connected, setConnected] = useState(false);
  const [exited, setExited] = useState<ExitedState | null>(null);
  const [displaced, setDisplaced] = useState(false);
  const [inputOverloaded, setInputOverloaded] = useState(false);
  const [fontSize, setFontSizeState] = useState(DEFAULT_FONT_SIZE);
  const [searchVisible, setSearchVisible] = useState(false);

  /** 键盘快捷键内部的字体调整（不进导出接口）。 */
  const changeFontSize = useCallback((size: number) => {
    const next = Math.min(MAX_FONT_SIZE, Math.max(MIN_FONT_SIZE, size));
    fontSizeRef.current = next;
    setFontSizeState(next);
  }, []);

  /** 发送 JSON 控制消息（仅在连接就绪时）。 */
  const sendControl = useCallback((msg: object) => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    }
  }, []);

  /** 统一的重置动作：清空恢复位置与屏幕，可选清除终端 id。 */
  const resetTerminalState = useCallback((clearId: boolean) => {
    lastSequenceRef.current = 0;
    termRef.current?.reset();
    if (clearId) {
      terminalIdRef.current = null;
    }
  }, []);

  /** rAF 防抖的自适应尺寸：fit 后把行列数上报给服务端。 */
  const scheduleFit = useCallback(() => {
    if (fitPendingRef.current) return;
    fitPendingRef.current = true;
    window.requestAnimationFrame(() => {
      fitPendingRef.current = false;
      const fit = fitRef.current;
      const term = termRef.current;
      if (!fit || !term || disposedRef.current) return;
      try {
        fit.fit();
      } catch {
        // 容器尺寸为 0（窗口最小化等）时忽略
      }
      const dims = fit.proposeDimensions();
      if (dims && dims.cols > 0 && dims.rows > 0) {
        sendControl({ type: "resize", cols: dims.cols, rows: dims.rows });
      }
    });
  }, [sendControl]);

  /** 字体变化后重新 fit 并上报尺寸。 */
  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    // 整体赋值（xterm options 为部分合并语义，且规避 Readonly 类型限制）
    term.options = { fontSize };
    scheduleFit();
  }, [fontSize, scheduleFit]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    disposedRef.current = false;

    const term = new Terminal({
      fontSize: DEFAULT_FONT_SIZE,
      fontFamily:
        '"SF Mono", Menlo, Monaco, "Cascadia Mono", "Courier New", monospace',
        cursorBlink: true,
        scrollback: 5000,
        theme: xtermTheme,
    });
    const fit = new FitAddon();
    const search = new SearchAddon({ highlightLimit: 1000 });
    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(new WebLinksAddon());
    term.open(container);
    fit.fit();
    termRef.current = term;
    fitRef.current = fit;
    searchRef.current = search;

    const isMac = /Mac|iPod|iPhone|iPad/.test(navigator.platform);

    async function paste() {
      try {
        const text = await navigator.clipboard.readText();
        if (text) term.paste(text);
      } catch {
        // 剪贴板权限被拒绝
      }
    }

    function copySelection() {
      const selection = term.getSelection();
      if (selection) {
        navigator.clipboard
          ?.writeText(selection)
          .catch(() => {}); // 剪贴板权限被拒绝时静默
      }
    }

    /**
     * 快捷键：
     * Ctrl/Cmd+Shift+C 复制、Ctrl/Cmd+Shift+V 粘贴、Ctrl/Cmd+F 搜索、
     * Ctrl/Cmd + +/-/0 字体缩放；macOS 下 Cmd+C/Cmd+V 直接复制粘贴。
     */
    function handleKeyDown(ev: KeyboardEvent): boolean {
      if (ev.type !== "keydown") return true;
      const mod = ev.ctrlKey || ev.metaKey;
      if (mod && ev.shiftKey && (ev.key === "C" || ev.key === "c")) {
        copySelection();
        return false;
      }
      if (mod && ev.shiftKey && (ev.key === "V" || ev.key === "v")) {
        void paste();
        return false;
      }
      if (isMac && ev.metaKey && !ev.shiftKey && (ev.key === "c" || ev.key === "C")) {
        // 终端惯例：有选中复制；无选中放行让 ^C 中断当前命令
        if (term.hasSelection()) {
          copySelection();
          return false;
        }
        return true;
      }
      if (isMac && ev.metaKey && (ev.key === "v" || ev.key === "V")) {
        void paste();
        return false;
      }
      if ((ev.ctrlKey || (isMac && ev.metaKey)) && !ev.shiftKey && (ev.key === "f" || ev.key === "F")) {
        setSearchVisible(true);
        return false;
      }
      if (mod && (ev.key === "+" || ev.key === "=")) {
        changeFontSize(fontSizeRef.current + 1);
        return false;
      }
      if (mod && (ev.key === "-" || ev.key === "_")) {
        changeFontSize(fontSizeRef.current - 1);
        return false;
      }
      if (mod && ev.key === "0") {
        changeFontSize(DEFAULT_FONT_SIZE);
        return false;
      }
      return true;
    }

    const encoder = new TextEncoder();
    function sendInput(data: string) {
      const ws = wsRef.current;
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      const payload = encoder.encode(data);
      const frame = new Uint8Array(payload.length + 1);
      frame[0] = 0x01;
      frame.set(payload, 1);
      ws.send(frame);
    }

    term.onData(sendInput);
    term.attachCustomKeyEventHandler(handleKeyDown);

    /** 服务端控制消息。 */
    function handleControl(raw: string) {
      let msg: Record<string, unknown>;
      try {
        msg = JSON.parse(raw) as Record<string, unknown>;
      } catch {
        return;
      }
      const type = typeof msg.type === "string" ? msg.type : "";
      switch (type) {
        case "connected": {
          reconnectAttemptRef.current = 0;
          setConnected(true);
          // 服务端尺寸可能与本地 fit 结果不同，以本地为准重新上报
          scheduleFit();
          sendControl({ type: "resume", last_sequence: lastSequenceRef.current });
          break;
        }
        case "ping": {
          sendControl({ type: "pong" });
          break;
        }
        case "pong":
          break;
        case "error": {
          if (msg.code === "OUTPUT_GAP") {
            // 缓冲已被淘汰：清屏后从头拉取保留的输出（有限恢复）
            resetTerminalState(false);
            sendControl({ type: "resume", last_sequence: 0 });
          } else if (msg.code === "INPUT_OVERFLOW") {
            setInputOverloaded(true);
            if (overflowTimerRef.current !== null) {
              window.clearTimeout(overflowTimerRef.current);
            }
            overflowTimerRef.current = window.setTimeout(() => {
              overflowTimerRef.current = null;
              setInputOverloaded(false);
            }, INPUT_OVERFLOW_NOTICE_MS);
          } else if (msg.code === "TERMINAL_EXITED") {
            stoppedRef.current = true;
            setExited({ code: null });
            wsRef.current?.close();
          }
          break;
        }
        case "exit": {
          const code = typeof msg.exit_code === "number" ? msg.exit_code : null;
          stoppedRef.current = true;
          setExited({ code });
          wsRef.current?.close();
          break;
        }
        default:
          break;
      }
    }

    function openSocket() {
      const protocol = location.protocol === "https:" ? "wss" : "ws";
      const ws = new WebSocket(`${protocol}://${location.host}/ws/terminal`);
      ws.binaryType = "arraybuffer";
      const generation = ++connGenRef.current;
      // 关闭被替换的旧连接（其 onclose 因代号不匹配不会触发重连）
      const previous = wsRef.current;
      if (previous && previous !== ws) previous.close();
      wsRef.current = ws;
      setConnected(false);

      ws.onopen = () => {
        lastServerMessageRef.current = Date.now();
      };

      ws.onmessage = (ev: MessageEvent) => {
        lastServerMessageRef.current = Date.now();
        if (typeof ev.data === "string") {
          handleControl(ev.data);
          return;
        }
        const bytes = new Uint8Array(ev.data);
        if (bytes.length < OUTPUT_HEADER_LEN || bytes[0] !== OUTPUT_TYPE) return;
        const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        const sequence = Number(view.getBigUint64(1));
        term.write(bytes.subarray(OUTPUT_HEADER_LEN));
        lastSequenceRef.current = sequence;
      };

      ws.onclose = (ev: CloseEvent) => {
        setConnected(false);
        if (wsRef.current === ws) wsRef.current = null;
        // 本连接已被更新的连接取代（restart/重连），其收尾事件不再驱动重连
        if (connGenRef.current !== generation) return;
        if (disposedRef.current) return;
        if (ev.code === CLOSE_REPLACED) {
          // 已被其他窗口接管：停止自动重连，避免两个窗口互踢
          setDisplaced(true);
          return;
        }
        if (stoppedRef.current) return;
        scheduleReconnect();
      };

      ws.onerror = () => {
        // onclose 会跟进
      };
    }

    function scheduleReconnect() {
      const attempt = reconnectAttemptRef.current;
      const delay = Math.min(MAX_RECONNECT_DELAY_MS, 1000 * 2 ** attempt);
      reconnectAttemptRef.current = attempt + 1;
      reconnectTimerRef.current = window.setTimeout(() => {
        reconnectTimerRef.current = null;
        connect();
      }, delay);
    }

    /** 先确保服务端终端存在（必要时重建已退出的终端），再建立 WebSocket。 */
    function connect() {
      if (disposedRef.current || stoppedRef.current || connectInFlightRef.current) return;
      connectInFlightRef.current = true;
      initTerminal()
        .then((info) => {
          connectInFlightRef.current = false;
          if (disposedRef.current || stoppedRef.current) return;
          if (terminalIdRef.current && terminalIdRef.current !== info.id) {
            // 终端被重新初始化（退出重建 / 应用重启），丢弃旧输出
            resetTerminalState(false);
          }
          terminalIdRef.current = info.id;
          openSocket();
        })
        .catch((err: unknown) => {
          connectInFlightRef.current = false;
          if (disposedRef.current) return;
          if (err instanceof ApiError && err.authExpired) {
            onAuthExpiredRef.current();
            return;
          }
          scheduleReconnect();
        });
    }

    connectRef.current = connect;
    connect();
    term.focus();

    // 页面尺寸变化时自动调整终端尺寸
    const observer = new ResizeObserver(() => scheduleFit());
    observer.observe(container);

    // 心跳：定期 ping 服务端
    const pingTimer = window.setInterval(() => {
      sendControl({ type: "ping" });
    }, PING_INTERVAL_MS);

    // 看门狗：服务端长时间无消息则主动断开触发重连
    const watchdogTimer = window.setInterval(() => {
      const ws = wsRef.current;
      if (
        ws &&
        ws.readyState === WebSocket.OPEN &&
        Date.now() - lastServerMessageRef.current > PONG_TIMEOUT_MS
      ) {
        ws.close();
      }
    }, 10_000);

    return () => {
      disposedRef.current = true;
      observer.disconnect();
      window.clearInterval(pingTimer);
      window.clearInterval(watchdogTimer);
      if (reconnectTimerRef.current !== null) {
        window.clearTimeout(reconnectTimerRef.current);
      }
      if (overflowTimerRef.current !== null) {
        window.clearTimeout(overflowTimerRef.current);
      }
      wsRef.current?.close();
      wsRef.current = null;
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      searchRef.current = null;
    };
    // 终端生命周期只初始化一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resetTerminalState, scheduleFit, sendControl]);

  const restart = useCallback(() => {
    stoppedRef.current = false;
    setExited(null);
    setDisplaced(false);
    resetTerminalState(true);
    connectRef.current();
  }, [resetTerminalState]);

  const search = useCallback((query: string, direction: 1 | -1) => {
    const addon = searchRef.current;
    if (!addon || !query) return;
    try {
      if (direction === 1) addon.findNext(query, { decorations: SEARCH_DECORATIONS });
      else addon.findPrevious(query, { decorations: SEARCH_DECORATIONS });
    } catch {
      // 无匹配时忽略
    }
  }, []);

  const closeSearch = useCallback(() => {
    searchRef.current?.clearDecorations();
    setSearchVisible(false);
    termRef.current?.focus();
  }, []);

  return {
    containerRef,
    connected,
    exited,
    displaced,
    inputOverloaded,
    searchVisible,
    search,
    closeSearch,
    restart,
  };
}
