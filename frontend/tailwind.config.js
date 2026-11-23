/** @type {import('tailwindcss').Config} */
export default {
  // 暗色分支：class 策略 + index.css .dark 令牌组已转正为暗色主题系统
  // （入口 main.ts 初始化 + App 顶栏 Sun/Moon 按钮切换）。
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
        // 色阶族：每个族 1..5 阶（3 = 基准，5 = 强调端；亮暗两套各自成阶），
        // 生成 bg-primary-3 / text-success-5 / border-destructive-1 等工具类
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
          1: "hsl(var(--primary-1))",
          2: "hsl(var(--primary-2))",
          3: "hsl(var(--primary-3))",
          4: "hsl(var(--primary-4))",
          5: "hsl(var(--primary-5))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
          1: "hsl(var(--secondary-1))",
          2: "hsl(var(--secondary-2))",
          3: "hsl(var(--secondary-3))",
          4: "hsl(var(--secondary-4))",
          5: "hsl(var(--secondary-5))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
          1: "hsl(var(--destructive-1))",
          2: "hsl(var(--destructive-2))",
          3: "hsl(var(--destructive-3))",
          4: "hsl(var(--destructive-4))",
          5: "hsl(var(--destructive-5))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
          1: "hsl(var(--muted-1))",
          2: "hsl(var(--muted-2))",
          3: "hsl(var(--muted-3))",
          4: "hsl(var(--muted-4))",
          5: "hsl(var(--muted-5))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
          1: "hsl(var(--accent-1))",
          2: "hsl(var(--accent-2))",
          3: "hsl(var(--accent-3))",
          4: "hsl(var(--accent-4))",
          5: "hsl(var(--accent-5))",
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
          foreground: "hsl(var(--success-foreground))",
          1: "hsl(var(--success-1))",
          2: "hsl(var(--success-2))",
          3: "hsl(var(--success-3))",
          4: "hsl(var(--success-4))",
          5: "hsl(var(--success-5))",
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
};
