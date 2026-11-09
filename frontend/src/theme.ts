/**
 * Dracula 主题单点定义。
 *
 * tailwind.config.js 中的 `dracula-*` 色板须与本文件保持一致；
 * xterm 主题由同一色板生成，组件样式请使用 `bg-dracula-*` 等工具类。
 */

export const dracula = {
  /** 终端/窗口背景 */
  bg: "#282a36",
  /** 标题栏背景 */
  bgDark: "#21222c",
  /** 登录页底色 */
  bgPage: "#191a21",
  /** 输入框底色 */
  input: "#1e2029",
  /** 描边 */
  border: "#44475a",
  /** 前景文字 */
  fg: "#f8f8f2",
  /** 注释/次要文字 */
  comment: "#6272a4",
  purple: "#bd93f9",
  pink: "#ff79c6",
  cyan: "#8be9fd",
  red: "#ff5555",
  green: "#50fa7b",
  yellow: "#f1fa8c",
  selection: "#44475a",
  selectionInactive: "#3a3d4d",
} as const;

/** xterm 主题（Dracula ANSI 16 色）。 */
export const xtermTheme = {
  background: dracula.bg,
  foreground: dracula.fg,
  cursor: dracula.fg,
  cursorAccent: dracula.bg,
  selectionBackground: dracula.selection,
  selectionForeground: dracula.fg,
  selectionInactiveBackground: dracula.selectionInactive,
  black: "#21222c",
  red: "#ff5555",
  green: "#50fa7b",
  yellow: "#f1fa8c",
  blue: "#bd93f9",
  magenta: "#ff79c6",
  cyan: "#8be9fd",
  white: "#f8f8f2",
  brightBlack: "#6272a4",
  brightRed: "#ff6e6e",
  brightGreen: "#69ff94",
  brightYellow: "#ffffa5",
  brightBlue: "#d6acff",
  brightMagenta: "#ff92df",
  brightCyan: "#a4ffff",
  brightWhite: "#ffffff",
} as const;
