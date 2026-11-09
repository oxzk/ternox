import { useState } from "react";
import type { FormEvent } from "react";
import Backdrop from "../components/Backdrop";
import { login } from "../services/api";

interface Props {
  onSuccess: () => void;
}

/** ANSI Shadow 风格 ASCII LOGO：TERNOX */
const ASCII_LOGO = String.raw`
 ████████╗███████╗██████╗ ███╗   ██╗ ██████╗ ██╗  ██╗
 ╚══██╔══╝██╔════╝██╔══██╗████╗  ██║██╔═══██╗╚██╗██╔╝
    ██║   █████╗  ██████╔╝██╔██╗ ██║██║   ██║ ╚███╔╝
    ██║   ██╔══╝  ██╔══██╗██║╚██╗██║██║   ██║ ██╔██╗
    ██║   ███████╗██║  ██║██║ ╚████║╚██████╔╝██╔╝ ██╗
    ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝  ╚═══╝ ╚═════╝ ╚═╝  ╚═╝`.trimStart();

/** 登录页面：Dracula 风格（ASCII LOGO + 光晕背景 + 玻璃卡片表单）。 */
export default function LoginPage({ onSuccess }: Props) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (ev: FormEvent) => {
    ev.preventDefault();
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await login(username.trim(), password);
      onSuccess();
    } catch (err) {
      setError(err instanceof Error ? err.message : "login failed");
      setPassword("");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="relative flex h-screen flex-col items-center justify-center overflow-hidden bg-dracula-bg-page px-4 font-mono">
      {/* 背景：点阵网格 + Dracula 紫/粉光晕 + CRT 扫描线 */}
      <Backdrop variant="dots" patternClassName="opacity-40" scanlineClassName="opacity-30" />
      <div className="pointer-events-none absolute -top-32 left-1/4 h-96 w-96 rounded-full bg-dracula-purple/15 blur-[130px]" />
      <div className="pointer-events-none absolute bottom-[-6rem] right-[12%] h-80 w-80 rounded-full bg-dracula-pink/10 blur-[120px]" />

      <div className="relative z-10 flex w-full max-w-md flex-col items-center">
        {/* ASCII LOGO */}
        <pre
          aria-label="TERNOX"
          className="login-glow-purple select-none text-center text-[7px] leading-[9px] text-dracula-purple sm:text-[11px] sm:leading-[13px]"
        >
          {ASCII_LOGO}
        </pre>
        <p className="mt-2 text-[11px] tracking-[0.35em] text-dracula-comment">
          SECURE SHELL GATEWAY
        </p>

        {/* 表单卡片 */}
        <form
          onSubmit={handleSubmit}
          className="mt-9 w-full rounded-xl border border-dracula-border bg-dracula-bg/70 p-7 shadow-[0_8px_40px_rgba(0,0,0,0.45)] backdrop-blur-md"
        >
          <div className="mb-6 flex items-center justify-between">
            <span className="text-[11px] tracking-[0.25em] text-dracula-comment">
              AUTHENTICATION
            </span>
            <span className="flex gap-1.5">
              <i className="h-1.5 w-1.5 rounded-full bg-dracula-pink/80" />
              <i className="h-1.5 w-1.5 rounded-full bg-dracula-purple/50" />
              <i className="h-1.5 w-1.5 rounded-full bg-dracula-cyan/40" />
            </span>
          </div>

          <label
            htmlFor="username"
            className="mb-1.5 block text-[11px] uppercase tracking-wider text-dracula-comment"
          >
            Username
          </label>
          <div className="mb-4 rounded-lg border border-dracula-border bg-dracula-input transition focus-within:border-dracula-purple focus-within:shadow-[0_0_0_3px_rgba(189,147,249,0.15)]">
            <input
              id="username"
              name="username"
              autoComplete="username"
              autoFocus
              spellCheck={false}
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              className="w-full rounded-lg bg-transparent px-3 py-2.5 text-sm text-dracula-fg caret-dracula-pink outline-none placeholder:text-dracula-comment/40"
            />
          </div>

          <label
            htmlFor="password"
            className="mb-1.5 block text-[11px] uppercase tracking-wider text-dracula-comment"
          >
            Password
          </label>
          <div className="mb-5 rounded-lg border border-dracula-border bg-dracula-input transition focus-within:border-dracula-purple focus-within:shadow-[0_0_0_3px_rgba(189,147,249,0.15)]">
            <input
              id="password"
              name="password"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="w-full rounded-lg bg-transparent px-3 py-2.5 text-sm text-dracula-fg caret-dracula-pink outline-none"
            />
          </div>

          {error && (
            <p className="login-shake login-glow-red mb-4 text-xs text-dracula-red">
              [!] {error}
            </p>
          )}

          <button
            type="submit"
            disabled={submitting || !username || !password}
            className="w-full rounded-lg bg-gradient-to-r from-dracula-pink to-dracula-purple py-2.5 text-sm font-medium tracking-[0.2em] text-dracula-bg transition hover:shadow-[0_0_28px_rgba(255,121,198,0.35)] disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:shadow-none"
          >
            {submitting ? "CONNECTING…" : "CONNECT →"}
          </button>
        </form>

        <p className="mt-5 text-[11px] tracking-wider text-dracula-comment/40">
          warning: unauthorized access is prohibited &amp; logged
        </p>
      </div>
    </div>
  );
}
