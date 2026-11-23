/**
 * 预览入口引导（仅 preview.html 引用，不进入生产 index.html）。
 * 先安装 Tauri 桥，再加载应用入口，保证 App 首次 IPC 前桥已就绪。
 */
import {
  installMockBridge,
  installRealBridge,
  showPreviewError,
} from "/ui-preview-init.js";

async function boot() {
  const params = new URLSearchParams(location.search);
  const mode = params.get("mode") === "real" ? "real" : "mock";
  // 允许截图/评审用 ?theme=dark|light 固定主题（App 在挂载前读 localStorage）
  const theme = params.get("theme");
  if (theme === "dark" || theme === "light") {
    localStorage.setItem("praxis-theme", theme);
  }
  const font = params.get("font");
  if (font && /^(16|17|18|19|20)$/.test(font)) {
    localStorage.setItem("praxis.readerFontSize", font);
  }
  try {
    if (mode === "real") {
      const response = await fetch("/preview-data.local.json", {
        cache: "no-store",
      });
      if (!response.ok) {
        throw new Error(
          `未找到本机真实数据（HTTP ${response.status}）。请先在仓库根运行只读导出脚本：agent-service/.venv/Scripts/python scripts/preview-export.py`,
        );
      }
      installRealBridge(await response.json());
    } else {
      installMockBridge();
    }
  } catch (error) {
    showPreviewError(error);
    return;
  }
  await import("/src/main.ts");
  applyDeepLink(params);
}

/**
 * 评审/截图辅助（仅预览）：用查询参数直接落到目标页面或计划详情。
 * ?page=calendar|notes|settings  切页
 * ?open=<planId>                  打开列表中的计划
 * ?scroll=<px>                    详情打开后滚动主区
 */
function applyDeepLink(params) {
  const page = params.get("page");
  if (page) {
    window.setTimeout(() => {
      document
        .querySelector(`.nav-item[aria-label="${pageLabel(page)}"]`)
        ?.click();
    }, 400);
  }
  const openId = params.get("open");
  if (!openId) return;
  const deadline = Date.now() + 6000;
  const timer = window.setInterval(() => {
    const record = (window.__previewRecords ?? []).find(
      (item) => item.id === openId,
    );
    const row = record
      ? document.querySelector(`[aria-label="查看计划：${record.title}"]`)
      : null;
    if (row) {
      window.clearInterval(timer);
      row.click();
      const scroll = Number(params.get("scroll"));
      if (Number.isFinite(scroll) && scroll > 0) {
        window.setTimeout(() => {
          const host = document.querySelector("main");
          if (host) host.scrollTop = scroll;
        }, 700);
      }
      return;
    }
    if (Date.now() > deadline) window.clearInterval(timer);
  }, 120);
}

function pageLabel(page) {
  return (
    {
      plan: "学习计划",
      study: "研读",
      calendar: "打卡日历",
      notes: "笔记生成",
      settings: "设置",
    }[page] ?? page
  );
}

void boot();
