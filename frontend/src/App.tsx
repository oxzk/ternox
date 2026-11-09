import { useCallback, useEffect, useState } from "react";
import Backdrop from "./components/Backdrop";
import LoginPage from "./pages/LoginPage";
import TerminalPage from "./pages/TerminalPage";
import { checkSession, logout } from "./services/api";

type AuthState = "checking" | "authenticated" | "guest";

export default function App() {
  const [auth, setAuth] = useState<AuthState>("checking");

  useEffect(() => {
    checkSession()
      .then((result) =>
        setAuth(result.authenticated ? "authenticated" : "guest"),
      )
      .catch(() => setAuth("guest"));
  }, []);

  const handleLogin = useCallback(() => setAuth("authenticated"), []);

  // 用户主动登出：销毁服务端 Session
  const handleLogout = useCallback(async () => {
    try {
      await logout();
    } catch {
      // 会话已失效等情况忽略
    } finally {
      setAuth("guest");
    }
  }, []);

  // 会话过期（401）：直接回登录页，无需调用注定失败的 logout
  const handleSessionExpired = useCallback(() => setAuth("guest"), []);

  if (auth === "checking") {
    return (
      <div className="relative flex h-screen items-center justify-center overflow-hidden bg-dracula-bg-page font-mono text-sm text-dracula-comment">
        <Backdrop
          variant="dots"
          patternClassName="opacity-30"
          scanlineClassName="opacity-40"
        />
        <span className="login-glow-purple relative z-10">
          connecting to ternox daemon…
        </span>
      </div>
    );
  }

  if (auth === "authenticated") {
    return (
      <TerminalPage
        onLogout={handleLogout}
        onSessionExpired={handleSessionExpired}
      />
    );
  }
  return <LoginPage onSuccess={handleLogin} />;
}
