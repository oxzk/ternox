import { useCallback } from "react";
import Backdrop from "../components/Backdrop";
import TerminalView from "../components/Terminal";
import { useTerminal } from "../hooks/useTerminal";

interface Props {
  /** 用户主动登出：销毁服务端 Session */
  onLogout: () => void;
  /** 会话过期（后端复验断开 → 重连 401）：直接回登录页 */
  onSessionExpired: () => void;
}

/** 终端页面：macOS 风格标题栏 + 全屏终端区域。 */
export default function TerminalPage({ onLogout, onSessionExpired }: Props) {
  const handle = useTerminal(onSessionExpired);

  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else {
      document.documentElement.requestFullscreen().catch(() => {});
    }
  }, []);

  return (
    <div className="relative flex h-screen w-screen flex-col overflow-hidden bg-dracula-bg-page font-mono">
      <Backdrop
        variant="grid"
        patternClassName="opacity-[0.05]"
        scanlineClassName="opacity-60"
      />

      <div className="relative z-10 flex min-h-0 flex-1 flex-col overflow-hidden bg-dracula-bg">
        {/* macOS 风格标题栏：控制点仅作装饰；双击标题栏切换全屏 */}
        <header
          onDoubleClick={toggleFullscreen}
          className="flex h-9 shrink-0 select-none items-center gap-2 border-b border-black/40 bg-dracula-bg-dark px-3"
        >
          <span className="h-3 w-3 rounded-full bg-[#ff5f57]" />
          <span className="h-3 w-3 rounded-full bg-[#febc2e]" />
          <span className="h-3 w-3 rounded-full bg-[#28c840]" />
          <span className="flex-1 text-center text-xs text-dracula-comment">
            ternox — bash
          </span>
          <div className="flex items-center gap-1">
            <button
              title="全屏（或双击标题栏）"
              onClick={toggleFullscreen}
              className="rounded p-1 text-sm text-dracula-comment hover:bg-white/10 hover:text-dracula-fg"
            >
              ⛶
            </button>
            <button
              title="退出登录（Shell 继续运行）"
              onClick={onLogout}
              className="rounded p-1 text-sm text-dracula-comment hover:bg-white/10 hover:text-dracula-fg"
            >
              ⏏
            </button>
          </div>
        </header>
        <TerminalView handle={handle} />
      </div>
    </div>
  );
}
