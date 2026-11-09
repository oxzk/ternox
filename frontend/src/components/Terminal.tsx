import { useEffect, useRef, useState } from "react";
import type { TerminalHandle } from "../hooks/useTerminal";

interface Props {
  handle: TerminalHandle;
}

/** 终端区域：xterm 容器 + 搜索栏 + 连接状态与退出/接管覆盖层。 */
export default function TerminalView({ handle }: Props) {
  const {
    containerRef,
    connected,
    exited,
    displaced,
    inputOverloaded,
    searchVisible,
    search,
    closeSearch,
    restart,
  } = handle;
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (searchVisible) inputRef.current?.focus();
  }, [searchVisible]);

  /** 搜索后保持焦点在输入框，避免 Cmd+F 落回浏览器、回车无法续搜。 */
  const searchAndFocus = (direction: 1 | -1) => {
    search(query, direction);
    inputRef.current?.focus();
  };

  return (
    <div className="relative min-h-0 flex-1 bg-dracula-bg">
      <div ref={containerRef} className="h-full w-full px-4 py-2" />

      {searchVisible && (
        <div className="absolute right-3 top-1 flex items-center gap-1 rounded-md border border-dracula-border bg-dracula-bg/95 px-2 py-1 shadow-lg">
          <input
            ref={inputRef}
            value={query}
            placeholder="搜索…"
            spellCheck={false}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                searchAndFocus(e.shiftKey ? -1 : 1);
                e.preventDefault();
              } else if (e.key === "Escape") {
                closeSearch();
              }
            }}
            className="w-44 bg-transparent text-sm text-dracula-fg outline-none placeholder:text-dracula-comment/50"
          />
          <button
            title="上一个 (Shift+Enter)"
            onClick={() => searchAndFocus(-1)}
            className="rounded px-1.5 text-dracula-comment hover:bg-white/10 hover:text-dracula-fg"
          >
            ↑
          </button>
          <button
            title="下一个 (Enter)"
            onClick={() => searchAndFocus(1)}
            className="rounded px-1.5 text-dracula-comment hover:bg-white/10 hover:text-dracula-fg"
          >
            ↓
          </button>
          <button
            title="关闭 (Esc)"
            onClick={closeSearch}
            className="rounded px-1.5 text-dracula-comment hover:bg-white/10 hover:text-dracula-fg"
          >
            ✕
          </button>
        </div>
      )}

      {!connected && !exited && !displaced && (
        <div className="pointer-events-none absolute bottom-3 right-4 rounded bg-dracula-input/90 px-2 py-1 text-xs text-dracula-comment">
          连接中…
        </div>
      )}

      {connected && inputOverloaded && (
        <div className="pointer-events-none absolute bottom-3 left-4 rounded bg-dracula-yellow/10 px-2 py-1 text-xs text-dracula-yellow">
          输入过载，部分按键可能丢失
        </div>
      )}

      {(displaced || exited) && (
        <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-4 bg-black/60 backdrop-blur-sm">
          <p className="text-sm text-dracula-fg">
            {displaced
              ? "终端已在其他窗口打开"
              : exited?.code != null
                ? `Shell 已退出（退出码 ${exited.code}）`
                : "Shell 已退出"}
          </p>
          <button
            onClick={restart}
            className="rounded-md bg-dracula-purple px-4 py-1.5 text-sm font-medium text-dracula-bg hover:bg-dracula-pink"
          >
            {displaced ? "接管终端" : "重新启动终端"}
          </button>
        </div>
      )}
    </div>
  );
}
