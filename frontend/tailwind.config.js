import animate from "tailwindcss-animate";

/** @type {import('tailwindcss').Config} */
export default {
  // [v1 清理] 重设计后源树零 dark: 引用，暗色分支成孤儿（index.css .dark 同批），
  // 删此行 + .dark 块 +下方 plugins 行（v0.1 G1 清理）
  darkMode: ["class"],
  content: ["./index.html", "./src/**/*.{vue,js,ts,jsx,tsx}"],
  theme: {
    container: {
      center: true,
      padding: "2rem",
      screens: { "2xl": "1400px" },
    },
    extend: {
      colors: {
        // shadcn/ui 标准 CSS 变量色板（hsl 通道形式，定义见 src/assets/index.css）
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        success: {
          DEFAULT: "hsl(var(--success))",
          foreground: "hsl(40 33% 98%)",
        },
      },
      fontFamily: {
        /* 书页标题：思源宋体优先，本机宋体兑底，全文仅用于标题角色 */
        display: [
          "Noto Serif SC",
          "Source Han Serif SC",
          "Songti SC",
          "SimSun",
          "Georgia",
          "serif",
        ],
      },
      borderRadius: {
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
      },
      keyframes: {
        "fade-in-up": {
          "0%": { opacity: "0", transform: "translateY(6px)" },
          // 终帧必须 none：both fill 会永久保留 100% 帧，任何非 none transform
          // 都会成为 fixed子孙（如 PlanResult 悬浮目录）的 containing block，
          // 把 fixed 钉进内容盒——这是「右缘竖线不见」的根因（目录修复）。
          "100%": { opacity: "1", transform: "none" },
        },
      },
      animation: {
        "fade-in-up": "fade-in-up 0.3s ease-out both",
      },
    },
  },
  // [v1 清理] 源树零 animate-in/out 工具类引用，tailwindcss-animate 成孤儿，删除随 G1
  plugins: [animate],
};
