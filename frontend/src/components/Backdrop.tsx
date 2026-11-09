interface Props {
  /** 背景纹理：点阵（登录页）或网格 */
  variant: "dots" | "grid";
  /** 纹理层透明度类 */
  patternClassName?: string;
  /** CRT 扫描线层透明度类 */
  scanlineClassName?: string;
}

/** 全局黑客风背景：纹理 + CRT 扫描线（登录页/终端页/检查页统一复用）。 */
export default function Backdrop({
  variant,
  patternClassName = "opacity-40",
  scanlineClassName = "opacity-30",
}: Props) {
  return (
    <>
      <div
        className={`${variant === "dots" ? "login-dots" : "hacker-grid"} pointer-events-none absolute inset-0 ${patternClassName}`}
      />
      <div
        className={`crt-lines pointer-events-none absolute inset-0 z-20 ${scanlineClassName}`}
      />
    </>
  );
}
