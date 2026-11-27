#!/usr/bin/env node
/**
 * 预览回归脚本（开发专用）：用本机 Chrome + CDP 驱动真实浏览器，验证
 * Mock 列表/阅读器/目录/生成流程与真实数据只读边界，并输出截图。
 *
 * 前提：dev server 已启动（默认 http://localhost:5201），命令：
 *   cd frontend && npx vite --port 5201 --strictPort
 * 运行：
 *   node scripts/preview-check.mjs
 * 可选环境变量：
 *   PREVIEW_URL=http://localhost:5201/preview.html
 *   CHROME_PATH=...（默认 Windows Chrome 路径）
 * 截图输出：.review/screens/check-*.png（本机忽略目录）
 */
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const BASE = process.env.PREVIEW_URL ?? "http://localhost:5201/preview.html";
const CHROME =
  process.env.CHROME_PATH ??
  "C:/Program Files/Google/Chrome/Application/chrome.exe";
const ROOT = path.resolve(process.cwd());
const OUT = path.join(ROOT, ".review", "screens");
const PORT = 9224;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const results = [];
function check(name, ok, detail) {
  results.push({ name, ok: Boolean(ok), detail });
  console.log(
    `${ok ? "PASS" : "FAIL"}  ${name}${detail === undefined ? "" : `  ${JSON.stringify(detail)}`}`,
  );
}

async function waitForDevServer() {
  for (let i = 0; i < 40; i += 1) {
    try {
      const response = await fetch(BASE);
      if (response.ok) return;
    } catch {
      // not up yet
    }
    await sleep(250);
  }
  throw new Error(`预览服务不可达：${BASE}（请先启动 frontend dev server）`);
}

async function main() {
  await fs.mkdir(OUT, { recursive: true });
  await waitForDevServer();
  const profile = await fs.mkdtemp(
    path.join(os.tmpdir(), "praxis-preview-check-"),
  );
  const chrome = spawn(
    CHROME,
    [
      "--headless=new",
      "--disable-gpu",
      "--no-first-run",
      `--remote-debugging-port=${PORT}`,
      `--user-data-dir=${profile}`,
      "--window-size=1440,1000",
      "about:blank",
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );

  let ws;
  let nextId = 1;
  const pending = new Map();
  const consoleWarnings = [];
  const dialogs = [];
  /** 下一次 confirm 的应答：dismiss | accept；未预设时默认取消 */
  let dialogDecision = "dismiss";
  const send = (method, params = {}) => {
    const id = nextId++;
    ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`CDP 超时：${method}`));
      }, 20000);
      pending.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
    });
  };
  const evaluate = async (expression) => {
    const out = await send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    });
    if (out.exceptionDetails)
      throw new Error(out.exceptionDetails.text ?? "页面脚本异常");
    return out.result?.value;
  };
  const shot = async (name) => {
    const data = await send("Page.captureScreenshot", {
      format: "png",
      captureBeyondViewport: false,
      fromSurface: true,
    });
    const file = path.join(OUT, `check-${name}.png`);
    await fs.writeFile(file, Buffer.from(data.data, "base64"));
    return file;
  };
  /** 研读改版交付截图：.review/study-*.png（与 check-* 分开命名） */
  const shotStudy = async (name) => {
    const data = await send("Page.captureScreenshot", {
      format: "png",
      captureBeyondViewport: false,
      fromSurface: true,
    });
    const file = path.join(ROOT, ".review", `study-${name}.png`);
    await fs.writeFile(file, Buffer.from(data.data, "base64"));
    return file;
  };
  const waitFor = async (expression, timeout = 8000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      if (await evaluate(expression)) return true;
      await sleep(150);
    }
    return false;
  };

  try {
    for (let i = 0; i < 100; i += 1) {
      try {
        const response = await fetch(`http://127.0.0.1:${PORT}/json/version`);
        if (response.ok) break;
      } catch {
        // wait
      }
      await sleep(100);
    }
    const created = await fetch(
      `http://127.0.0.1:${PORT}/json/new?about:blank`,
      {
        method: "PUT",
      },
    );
    const target = await created.json();
    ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", reject, { once: true });
    });
    ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (
        message.method === "Runtime.consoleAPICalled" &&
        message.params?.type === "warning"
      ) {
        consoleWarnings.push(
          (message.params.args ?? [])
            .map((arg) => arg.value ?? arg.description ?? "")
            .join(" "),
        );
      }
      if (message.method === "Page.javascriptDialogOpening") {
        dialogs.push({ ...message.params, decision: dialogDecision });
        send("Page.handleJavaScriptDialog", {
          accept: dialogDecision === "accept",
        }).catch((error) => console.error("dialog reply:", error.message));
        dialogDecision = "dismiss";
        return;
      }
      if (!message.id || !pending.has(message.id)) return;
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    });

    // 无系统焦点时 Chromium 抑制 DOM focus/blur 事件（element.focus() 仍会
    // 移动 activeElement 但不派发事件），键盘路径断言前必须先把页面带到前台
    await send("Page.bringToFront");
    await send("Page.enable");
    await send("Runtime.enable");
    await send("Emulation.setDeviceMetricsOverride", {
      width: 1440,
      height: 1000,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await send("Emulation.setEmulatedMedia", {
      features: [{ name: "prefers-reduced-motion", value: "reduce" }],
    });

    const goto = async (url) => {
      await send("Page.navigate", { url });
      await waitFor("document.readyState === 'complete'");
      await sleep(800);
    };

    // 受信任键盘事件（合成 KeyboardEvent 不触发浏览器隐式表单提交等默认行为）
    const pressKey = async (key, keyCode) => {
      for (const type of ["keyDown", "keyUp"]) {
        await send("Input.dispatchKeyEvent", {
          type,
          key,
          code: key,
          windowsVirtualKeyCode: keyCode,
          nativeVirtualKeyCode: keyCode,
        });
      }
    };

    /** 受信任鼠标点击（原生 confirm 等默认行为需要真实输入事件） */
    const clickSelector = async (selector) => {
      const center = await evaluate(`(() => {
        const el = document.querySelector(${JSON.stringify(selector)});
        if (!el) return null;
        el.scrollIntoView({ block: 'center' });
        const r = el.getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
      })()`);
      if (!center) throw new Error(`点击目标不存在：${selector}`);
      await send("Input.dispatchMouseEvent", {
        type: "mouseMoved",
        x: center.x,
        y: center.y,
      });
      await send("Input.dispatchMouseEvent", {
        type: "mousePressed",
        x: center.x,
        y: center.y,
        button: "left",
        clickCount: 1,
      });
      await send("Input.dispatchMouseEvent", {
        type: "mouseReleased",
        x: center.x,
        y: center.y,
        button: "left",
        clickCount: 1,
      });
    };

    // ---- Mock：列表与搜索 ----
    await goto(`${BASE}?mode=mock&theme=light`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('学习计划')",
    );
    check(
      "mock 列表渲染",
      (await evaluate(
        "document.querySelectorAll('[aria-label^=\"查看计划\"]').length",
      )) === 4,
    );
    check("列表截图", Boolean(await shot("library-light")));
    // 色阶令牌：六族 × 5 阶在亮色主题下可用，且 3 阶与族基准同值
    check(
      "色阶令牌亮色可用",
      await evaluate(
        `(() => {
          const root = getComputedStyle(document.documentElement);
          const val = (v) => root.getPropertyValue(v).trim();
          const ok = ['primary','secondary','accent','muted','destructive','success']
            .every((c) => [1,2,3,4,5].every((n) => val('--' + c + '-' + n) !== ''));
          return ok && val('--primary-3') === val('--primary');
        })()`,
      ),
    );
    // 侧栏 tooltip 仅在标签不可见（折叠）时启用（评审确认冗余错位）。
    // 注意：气泡是模块级单例节点（不可移除 DOM 探测），浮出延迟 300ms。
    const tipProbe = async () => {
      await evaluate(`(() => {
        const item = [...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('打卡日历'));
        item?.dispatchEvent(new MouseEvent('mouseenter', { bubbles: false }));
      })()`);
      await sleep(600);
      const shown = await evaluate(
        "!!document.querySelector('.praxis-tip--show')",
      );
      await evaluate(
        `(() => {
          const item = [...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('打卡日历'));
          item?.dispatchEvent(new MouseEvent('mouseleave', { bubbles: false }));
        })()`,
      );
      await sleep(300);
      return shown;
    };
    const expandedTip = await tipProbe();
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.includes('收起侧栏'))?.click()",
    );
    await sleep(300);
    const collapsedTip = await tipProbe();
    check("展开态无侧栏 tooltip", expandedTip === false, expandedTip);
    check("折叠态有侧栏 tooltip", collapsedTip === true, collapsedTip);
    // 点击立即收起；键盘聚焦显示、失焦收起
    await evaluate(`(() => {
      const item = [...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('打卡日历'));
      item?.dispatchEvent(new MouseEvent('mouseenter', { bubbles: false }));
    })()`);
    await sleep(400);
    const tipBeforeClick = await evaluate(
      "!!document.querySelector('.praxis-tip--show')",
    );
    await evaluate(`(() => {
      const item = [...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('打卡日历'));
      item?.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    })()`);
    await sleep(150);
    check(
      "tooltip 点击立即收起",
      tipBeforeClick &&
        !(await evaluate("!!document.querySelector('.praxis-tip--show')")),
    );
    await evaluate(`(() => {
      const item = [...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('打卡日历'));
      item?.focus();
    })()`);
    await sleep(450);
    const tipOnFocus = await evaluate(
      "!!document.querySelector('.praxis-tip--show')",
    );
    await evaluate("document.activeElement?.blur()");
    await sleep(150);
    check(
      "tooltip 键盘聚焦显示且失焦收起",
      tipOnFocus &&
        !(await evaluate("!!document.querySelector('.praxis-tip--show')")),
    );
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.includes('展开菜单') || b.getAttribute('aria-label') === '展开菜单')?.click()",
    );
    await sleep(300);
    await evaluate(`(() => {
      const input = document.querySelector('input[type=search]');
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      set.call(input, 'Web');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await sleep(650);
    check(
      "搜索过滤",
      (await evaluate(
        "document.querySelectorAll('[aria-label^=\"查看计划\"]').length",
      )) === 1,
    );
    await evaluate(`(() => {
      const input = document.querySelector('input[type=search]');
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      set.call(input, 'zzzz-no-match');
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    check(
      "搜索空态",
      await waitFor(
        "document.body.innerText.includes('没有找到这份计划')",
        4000,
      ),
    );
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '清空搜索')?.click()",
    );
    await sleep(650);
    check(
      "清空搜索恢复列表",
      (await evaluate(
        "document.querySelectorAll('[aria-label^=\"查看计划\"]').length",
      )) === 4,
    );

    // ---- 列表写操作（内存 mock）：收藏 / 分组 / 删除取消 ----
    await evaluate(
      "document.querySelector('[aria-label=\"取消收藏：Python 编程：从理解到独立实践\"]')?.click()",
    );
    await sleep(200);
    check(
      "取消收藏",
      await evaluate(
        "!!document.querySelector('[aria-label=\"收藏：Python 编程：从理解到独立实践\"]')",
      ),
    );
    await evaluate(
      "document.querySelector('[aria-label=\"收藏：Python 编程：从理解到独立实践\"]')?.click()",
    );
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '新建分组')?.click()",
    );
    await sleep(150);
    await evaluate(`(() => {
      const input = document.querySelector('input[aria-label="新分组名称"]');
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      set.call(input, '临时分组');
      input.dispatchEvent(new Event('input', { bubbles: true }));
      [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '创建分组')?.click();
    })()`);
    await sleep(400);
    await evaluate(
      "document.querySelector('[aria-label=\"重命名分组：临时分组\"]')?.click()",
    );
    await sleep(150);
    await evaluate(`(() => {
      const input = document.querySelector('input[aria-label="分组新名称"]');
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      set.call(input, '改名分组');
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.focus();
      // IME 选词确认的 Enter（isComposing）不应提交
      input.dispatchEvent(new KeyboardEvent('keydown', {
        key: 'Enter', code: 'Enter', bubbles: true, cancelable: true, isComposing: true,
      }));
    })()`);
    await sleep(250);
    check(
      "IME 组合中的 Enter 不提交重命名",
      await evaluate(
        "!!document.querySelector('input[aria-label=\"分组新名称\"]')",
      ),
    );
    await pressKey("Enter", 13);
    await sleep(400);
    check(
      "分组创建与重命名",
      await evaluate(
        "!!document.querySelector('[aria-label=\"删除分组：改名分组\"]')",
      ),
    );
    // Escape 取消重命名，不提交当前草稿
    await evaluate(
      "document.querySelector('[aria-label=\"重命名分组：改名分组\"]')?.click()",
    );
    await sleep(150);
    await evaluate(`(() => {
      const input = document.querySelector('input[aria-label="分组新名称"]');
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      set.call(input, '不应保存的名字');
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.focus();
    })()`);
    await pressKey("Escape", 27);
    await sleep(300);
    check(
      "Escape 取消重命名",
      (await evaluate(
        "!document.querySelector('input[aria-label=\"分组新名称\"]')",
      )) &&
        (await evaluate(
          "!!document.querySelector('[aria-label=\"删除分组：改名分组\"]')",
        )),
    );
    await evaluate(
      "document.querySelector('[aria-label=\"移动到分组：理解 Web：从浏览器到服务端\"]')?.click()",
    );
    await sleep(200);
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '改名分组')?.click()",
    );
    await sleep(500);
    check(
      "移动计划到分组",
      await evaluate(
        "(document.querySelector('[aria-label=\"删除分组：改名分组\"]')?.closest('section')?.innerText || '').includes('理解 Web')",
      ),
    );
    // 删除使用原生 confirm：必须保留 CDP 连接并应答 javascriptDialogOpening，
    // 不能用 window.confirm 桩冒充（模拟用户曾因未处理对话框误判为冻结）。
    dialogDecision = "dismiss";
    await clickSelector('[aria-label="删除分组：改名分组"]');
    await sleep(600);
    check(
      "删除分组可取消（真实确认框）",
      (await evaluate(
        "!!document.querySelector('[aria-label=\"删除分组：改名分组\"]')",
      )) &&
        dialogs.some(
          (dialog) =>
            dialog.type === "confirm" &&
            dialog.message.includes("删除分组") &&
            dialog.decision === "dismiss",
        ),
    );
    dialogDecision = "accept";
    await clickSelector('[aria-label="删除分组：改名分组"]');
    await sleep(700);
    check(
      "删除分组确认后计划回到未分组（真实确认框）",
      await evaluate(
        "!document.querySelector('[aria-label=\"删除分组：改名分组\"]') && document.body.innerText.includes('未分组')",
      ),
    );
    dialogDecision = "dismiss";
    await clickSelector('[aria-label="删除：设计的日常：视觉与信息秩序"]');
    await sleep(600);
    check(
      "删除计划可取消（真实确认框）",
      (await evaluate(
        "document.querySelectorAll('[aria-label^=\"查看计划\"]').length",
      )) === 4,
    );
    dialogDecision = "accept";
    await clickSelector('[aria-label="删除：设计的日常：视觉与信息秩序"]');
    await sleep(700);
    check(
      "删除计划确认后移除（真实确认框）",
      (await evaluate(
        "document.querySelectorAll('[aria-label^=\"查看计划\"]').length",
      )) === 3,
    );

    // ---- Mock：阅读器排版 ----
    await goto(`${BASE}?mode=mock&theme=light&font=17&open=preview-plan-1`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('Python 编程')",
      10000,
    );
    const typography = await evaluate(`(() => {
      const p = document.querySelector('.markdown-code p');
      const s = getComputedStyle(p);
      return {
        font: parseFloat(s.fontSize),
        ratio: +(parseFloat(s.lineHeight) / parseFloat(s.fontSize)).toFixed(2),
        width: Math.round(document.querySelector('.reader-body').getBoundingClientRect().width),
      };
    })()`);
    check("正文字号 17px", typography.font === 17, typography);
    check(
      "行高 1.85–1.95",
      typography.ratio >= 1.85 && typography.ratio <= 1.95,
      typography,
    );
    check(
      "阅读列宽 680–760",
      typography.width >= 680 && typography.width <= 760,
      typography.width,
    );
    check("阅读器截图", Boolean(await shot("reader-light")));
    check(
      "代码内假标题不进入章目录",
      await evaluate(
        "!(document.querySelector('nav[aria-label=\"本章目录\"]')?.innerText || '').includes('代码里的假标题')",
      ),
    );

    await evaluate(`(() => {
      const main = document.querySelector('main');
      const el = document.querySelector('.markdown-code pre');
      main.scrollTo({ top: el.getBoundingClientRect().top - main.getBoundingClientRect().top + main.scrollTop - 140, behavior: 'instant' });
    })()`);
    await sleep(400);
    check(
      "宽表局部横滚",
      await evaluate(
        "(() => { const t = document.querySelector('.table-scroll'); return t.scrollWidth > t.clientWidth; })()",
      ),
    );
    check(
      "宽表溢出提示可见",
      await evaluate(
        "(() => { const t = document.querySelector('.table-scroll'); const hint = t.querySelector('.table-hint'); return t.classList.contains('is-overflow') && !!hint && getComputedStyle(hint).display !== 'none'; })()",
      ),
    );
    check(
      "未溢出表格无提示",
      await evaluate(
        "[...document.querySelectorAll('.table-scroll')].filter((t) => !t.classList.contains('is-overflow')).every((t) => getComputedStyle(t.querySelector('.table-hint')).display === 'none')",
      ),
    );
    check(
      "代码长行局部横滚",
      await evaluate(
        "[...document.querySelectorAll('.code-block pre')].some((p) => p.scrollWidth > p.clientWidth)",
      ),
    );

    await evaluate(`(() => {
      const main = document.querySelector('main');
      const el = document.querySelector('figure');
      main.scrollTo({ top: el.getBoundingClientRect().top - main.getBoundingClientRect().top + main.scrollTop - 110, behavior: 'instant' });
    })()`);
    await sleep(2500);
    check(
      "Mermaid 成功渲染",
      await evaluate("!!document.querySelector('figure svg')"),
    );
    check(
      "Mermaid 亮色用项目色板",
      await evaluate(
        "(document.querySelector('figure svg')?.outerHTML ?? '').includes('hsl(208, 30%, 94%)')",
      ),
    );
    check(
      "Mermaid 画布随短图自适应",
      await evaluate(
        "(() => { const v = document.querySelector('[aria-roledescription=\"可缩放图示\"]'); const svg = v?.querySelector('svg'); if (!v || !svg) return false; return v.getBoundingClientRect().height < 320; })()",
      ),
    );
    check("Mermaid 截图", Boolean(await shot("reader-mermaid")));
    const zoomBefore = await evaluate(
      "document.querySelector('figure .w-10')?.textContent.trim()",
    );
    await evaluate(
      "document.querySelector('figure [aria-label=\"放大\"]')?.click()",
    );
    await sleep(300);
    const zoomAfter = await evaluate(
      "document.querySelector('figure .w-10')?.textContent.trim()",
    );
    check("Mermaid 可缩放", zoomBefore !== zoomAfter, {
      zoomBefore,
      zoomAfter,
    });

    const invalidState = await evaluate(`(() => {
      const chapter = document.querySelector('.reader-chapter');
      const figures = chapter ? [...chapter.querySelectorAll('figure')] : [];
      const invalid = figures[1];
      return { count: figures.length, lazyPlaceholder: invalid ? !invalid.querySelector('svg') : null };
    })()`);
    await evaluate(`(() => {
      const chapter = document.querySelector('.reader-chapter');
      const invalid = chapter?.querySelectorAll('figure')[1];
      if (!invalid) return;
      const main = document.querySelector('main');
      main.scrollTo({ top: invalid.getBoundingClientRect().top - main.getBoundingClientRect().top + main.scrollTop - 110, behavior: 'instant' });
    })()`);
    await sleep(2000);
    check(
      "Mermaid 失败降级为源码",
      await evaluate(
        "document.body.innerText.includes('图示渲染失败，显示原始源码')",
      ),
    );
    check(
      "Mermaid 懒渲染（未进视口不渲染）",
      invalidState.count < 2 || invalidState.lazyPlaceholder === true,
      invalidState,
    );

    // 转义括号图示：首次渲染失败后应引号化修复成图，而不是降级源码
    await evaluate(`(() => {
      const h = [...document.querySelectorAll('.markdown-code h3')].find((x) => x.textContent.includes('转义括号的自动修复'));
      if (!h) return;
      const main = document.querySelector('main');
      main.scrollTo({ top: h.getBoundingClientRect().top - main.getBoundingClientRect().top + main.scrollTop - 140, behavior: 'instant' });
    })()`);
    await sleep(2500);
    const repairFigure = await evaluate(`(() => {
      const h = [...document.querySelectorAll('.markdown-code h3')].find((x) => x.textContent.includes('转义括号的自动修复'));
      if (!h) return null;
      const fig = h.closest('.reader-chapter')?.querySelectorAll('figure')[0]
        ?? h.parentElement?.parentElement?.querySelectorAll('figure')[0];
      if (!fig) return null;
      return { svg: !!fig.querySelector('svg'), degraded: fig.textContent.includes('图示渲染失败') };
    })()`);
    check(
      "Mermaid 转义括号自动修复成图",
      !!repairFigure && repairFigure.svg && !repairFigure.degraded,
      repairFigure,
    );

    await evaluate(`(() => {
      const main = document.querySelector('main');
      const h = [...document.querySelectorAll('.markdown-code h2')].find((x) => x.textContent.includes('图片与长内容'));
      main.scrollTo({ top: h.getBoundingClientRect().top - main.getBoundingClientRect().top + main.scrollTop - 100, behavior: 'instant' });
    })()`);
    await sleep(1200);
    check(
      "图片失败占位",
      await evaluate(
        "document.body.innerText.includes('图片未能加载：缺失的示意图')",
      ),
    );
    check(
      "危险 HTML 未执行",
      (await evaluate("window.__praxisXss")) === undefined,
    );

    // 代码复制反馈（成功 / 失败）
    await evaluate(`(() => {
      window.__clip = 'ok';
      Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => {} } });
      const btn = document.querySelector('.code-copy');
      btn.disabled = false;
      btn.click();
    })()`);
    await sleep(120);
    check(
      "复制成功反馈",
      (await evaluate(
        "document.querySelector('.code-copy')?.textContent.trim()",
      )) === "已复制",
    );
    await sleep(2100);
    await evaluate(`(() => {
      Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => { throw new Error('denied'); } } });
      document.querySelector('.code-copy').click();
    })()`);
    await sleep(150);
    check(
      "复制失败反馈",
      (await evaluate(
        "document.querySelector('.code-copy')?.textContent.trim()",
      )) === "复制失败，请重试",
    );

    // ---- 目录 / 专注 ----
    await evaluate("document.querySelector('main').scrollTop = 0");
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '目录')?.click()",
    );
    await sleep(300);
    check(
      "目录打开并聚焦",
      await evaluate("document.activeElement?.id === 'reader-toc'"),
    );
    await shot("reader-toc");
    await evaluate(
      "document.querySelector('#reader-toc').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
    );
    await sleep(200);
    check(
      "Escape 关闭目录并回焦",
      await evaluate(
        "!document.querySelector('#reader-toc') && document.activeElement?.getAttribute('aria-controls') === 'reader-toc'",
      ),
    );
    // 焦点在正文（面板外）时，全局 Escape 也要能关目录
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '目录')?.click()",
    );
    await sleep(250);
    await evaluate(
      "document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
    );
    await sleep(250);
    check(
      "正文焦点 Escape 关闭目录",
      await evaluate("!document.querySelector('#reader-toc')"),
    );
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '目录')?.click()",
    );
    await sleep(200);
    await evaluate(
      "[...document.querySelectorAll('#reader-toc nav > div > button')][1]?.click()",
    );
    await sleep(1200);
    check(
      "目录跨阶段跳转且不动根滚动",
      await evaluate(
        "window.scrollY === 0 && document.querySelector('#reader-toc [aria-current]')?.textContent.includes('2.')",
      ),
    );
    await evaluate(
      "document.querySelector('#reader-toc').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
    );
    await evaluate("document.querySelector('main').scrollTop = 0");
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.includes('专注阅读'))?.click()",
    );
    await sleep(300);
    check(
      "专注模式隐藏侧栏顶栏",
      await evaluate(
        "getComputedStyle(document.querySelector('aside')).display === 'none' && getComputedStyle(document.querySelector('header')).display === 'none'",
      ),
    );
    check(
      "专注模式可退出",
      await evaluate(
        "[...document.querySelectorAll('button')].some((b) => b.textContent.includes('退出专注'))",
      ),
    );
    await shot("reader-focus");
    // 焦点在正文（中性区域）时 Escape 也要能退出专注
    await evaluate(
      "document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
    );
    await sleep(300);
    check(
      "Escape 退出专注",
      await evaluate(
        "getComputedStyle(document.querySelector('aside')).display !== 'none'",
      ),
    );
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.includes('专注阅读'))?.click()",
    );
    await sleep(200);
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.includes('退出专注'))?.click()",
    );
    // 阅读器内侧栏「学习计划」应能返回列表（评审确认缺陷）
    await evaluate(
      "[...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('学习计划'))?.click()",
    );
    await sleep(500);
    check(
      "阅读器内点侧栏返回列表",
      await evaluate(
        "document.querySelector('h1')?.textContent.includes('学习计划')",
      ),
    );

    // 新建视图内点侧栏「学习计划」同样回到列表
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '新建计划')?.click()",
    );
    await sleep(300);
    await evaluate(
      "[...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('学习计划'))?.click()",
    );
    await sleep(400);
    check(
      "新建视图内点侧栏返回列表",
      await evaluate(
        "document.querySelector('h1')?.textContent.includes('学习计划')",
      ),
    );

    // 生成中离开页面，后台继续；回到列表可见新计划并能再次打开
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '新建计划')?.click()",
    );
    await sleep(300);
    await evaluate(`(() => {
      const setText = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      const goal = document.querySelector('#goal');
      setText.call(goal, '离开页面后继续生成');
      goal.dispatchEvent(new Event('input', { bubbles: true }));
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      for (const [id, value] of [['weekly-hours', 8], ['weeks', 6]]) {
        const el = document.getElementById(id);
        set.call(el, String(value));
        el.dispatchEvent(new Event('input', { bubbles: true }));
      }
      [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '生成学习计划')?.click();
    })()`);
    await waitFor("document.getElementById('generation-flow')", 4000);
    await evaluate(
      "[...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('学习计划'))?.click()",
    );
    await sleep(500);
    check(
      "生成中离开回到列表",
      await evaluate(
        "document.querySelector('h1')?.textContent.includes('学习计划')",
      ),
    );
    check(
      "离开期间后台生成完成并入列表",
      await waitFor(
        "document.body.innerText.includes('离开页面后继续生成') && (window.__previewRecords || []).some((r) => r.title.includes('离开页面后继续生成') && r.status === 'complete')",
        12000,
      ),
    );
    await evaluate(
      "document.querySelector('[aria-label=\"查看计划：离开页面后继续生成\"]')?.click()",
    );
    check(
      "完成后重新打开生成结果",
      await waitFor(
        "document.querySelector('h1')?.textContent.includes('离开页面后继续生成') && !!document.querySelector('.reader-chapter')",
        8000,
      ),
    );
    await evaluate(
      "[...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('学习计划'))?.click()",
    );
    await sleep(400);

    // ---- 表单校验与生成 ----
    await goto(`${BASE}?mode=mock&theme=light`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('学习计划')",
    );
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '新建计划')?.click()",
    );
    await sleep(300);
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '生成学习计划')?.click()",
    );
    await sleep(300);
    check(
      "空目标校验与聚焦",
      await evaluate(
        "document.querySelector('#goal-error')?.textContent.includes('至少用 4 个字') && document.activeElement?.id === 'goal'",
      ),
    );
    await shot("validation");
    await evaluate(`(() => {
      window.__resultFocusCount = 0;
      document.addEventListener('focusin', (event) => {
        if (event.target?.id === 'plan-result-title') window.__resultFocusCount += 1;
      }, true);
      const setText = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      const goal = document.querySelector('#goal');
      setText.call(goal, '学习数据可视化与叙事表达');
      goal.dispatchEvent(new Event('input', { bubbles: true }));
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      for (const [id, value] of [['weekly-hours', 8], ['weeks', 2]]) {
        const el = document.getElementById(id);
        set.call(el, String(value));
        el.dispatchEvent(new Event('input', { bubbles: true }));
      }
      [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '生成学习计划')?.click();
    })()`);
    check(
      "生成过程骨架可见",
      await waitFor(
        "document.body.innerText.includes('阶段完成') || document.body.innerText.includes('正在编排')",
        4000,
      ),
    );
    check(
      "提交后生成区进入视口",
      await evaluate(
        "(() => { const el = document.getElementById('generation-flow'); if (!el) return false; const r = el.getBoundingClientRect(); return r.top < window.innerHeight && r.bottom > 0; })()",
      ),
    );
    check(
      "生成终态进入结果页",
      await waitFor(
        "document.querySelector('h1')?.textContent.includes('学习数据可视化')",
        12000,
      ),
    );
    check(
      "生成完成结果标题进入视口",
      await evaluate(
        "(() => { const el = document.getElementById('plan-result-top'); if (!el) return false; const r = el.getBoundingClientRect(); return r.top >= 0 && r.top < window.innerHeight; })()",
      ),
    );
    check(
      "生成完成单次聚焦结果标题",
      (await evaluate("document.activeElement?.id")) === "plan-result-title" &&
        (await evaluate("window.__resultFocusCount")) === 1,
      {
        active: await evaluate("document.activeElement?.id"),
        count: await evaluate("window.__resultFocusCount"),
      },
    );
    check(
      "生成完成有可访问状态反馈",
      await evaluate(
        "document.querySelector('[role=status]')?.textContent.includes('已生成')",
      ),
    );
    await sleep(900);
    check(
      "完成后不重复抢焦点",
      (await evaluate("window.__resultFocusCount")) === 1,
    );
    check(
      "生成记录已入列表",
      await evaluate("(window.__previewRecords || []).length === 5"),
    );

    // ---- 研读改版：大纲-only 生成流 + 周时间线 + 去研读入口 ----
    check(
      "大纲-only 生成流产出周时间线",
      await waitFor("!!document.querySelector('[data-week-timeline]')", 4000),
    );
    check(
      "周时间线含周次徽标与学时合计",
      await evaluate(
        "(document.querySelector('[data-week-timeline]')?.textContent ?? '').includes('第 1 周') && (document.querySelector('[data-week-timeline]')?.textContent ?? '').includes('≈')",
      ),
    );
    check(
      "mock 大纲周次随表单缩放（weeks=2 不越界）",
      await evaluate(
        "(() => { const t = document.querySelector('[data-week-timeline]')?.textContent ?? ''; const weeks = [...t.matchAll(/第 (\\d+) 周/g)].map((m) => Number(m[1])); return weeks.length > 0 && Math.max(...weeks) <= 2; })()",
      ),
    );
    check(
      "周时间线章节 chips 可点击",
      (await evaluate(
        "document.querySelectorAll('[data-week-timeline] button').length",
      )) > 0,
    );
    await shotStudy("week-timeline");
    check(
      "结果页章节显示未研读态",
      await evaluate(
        "(document.querySelector('.reader-chapter')?.textContent ?? '').includes('未研读')",
      ),
    );
    check(
      "结果页提供去研读入口",
      await evaluate("!!document.querySelector('[data-study-goto]')"),
    );
    await evaluate("document.querySelector('[data-study-goto]')?.click()");
    check(
      "去研读跳转研读页并预选计划",
      await waitFor(
        "document.querySelector('h1')?.textContent.includes('研读') && (document.querySelector('[data-study-plan-select]')?.selectedOptions?.[0]?.textContent ?? '').includes('学习数据可视化')",
        8000,
      ),
    );
    check(
      "去研读携带章节直达（aria-current 落在源章节）",
      await waitFor(
        "document.querySelector('h1')?.textContent.includes('研读') && document.querySelector('[data-study-chapter][aria-current=\"true\"]')?.getAttribute('data-study-chapter') === '0:0'",
        6000,
      ),
    );

    // 失败场景：保留表单/输入，只在下方显示错误
    await goto(`${BASE}?mode=mock&theme=light&gen=fail`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('学习计划')",
    );
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '新建计划')?.click()",
    );
    await sleep(250);
    await evaluate(`(() => {
      const setText = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      const goal = document.querySelector('#goal');
      setText.call(goal, '失败时保留输入');
      goal.dispatchEvent(new Event('input', { bubbles: true }));
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      for (const [id, value] of [['weekly-hours', 8], ['weeks', 6]]) {
        const el = document.getElementById(id);
        set.call(el, String(value));
        el.dispatchEvent(new Event('input', { bubbles: true }));
      }
      [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '生成学习计划')?.click();
    })()`);
    check(
      "生成失败显示错误且保留表单",
      await waitFor(
        "document.body.innerText.includes('模拟生成失败') && document.querySelector('#goal')?.value === '失败时保留输入' && !document.querySelector('#plan-result-title')",
        5000,
      ),
    );
    check(
      "生成失败有读屏反馈",
      await evaluate(
        "document.querySelector('[role=status]')?.textContent.includes('未提交')",
      ),
    );
    await shot("generation-failure");

    // ---- 暗色 ----
    await send("Emulation.setDeviceMetricsOverride", {
      width: 1440,
      height: 1000,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await goto(`${BASE}?mode=mock&theme=dark`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('学习计划')",
    );
    await shot("library-dark");
    await goto(`${BASE}?mode=mock&theme=dark&font=17&open=preview-plan-1`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('Python 编程')",
      10000,
    );
    await evaluate(`(() => {
      const main = document.querySelector('main');
      const el = document.querySelector('figure');
      main.scrollTo({ top: el.getBoundingClientRect().top - main.getBoundingClientRect().top + main.scrollTop - 110, behavior: 'instant' });
    })()`);
    await sleep(2500);
    check(
      "暗色 Mermaid 渲染",
      await evaluate("!!document.querySelector('figure svg')"),
    );
    check(
      "Mermaid 暗色用项目色板",
      await evaluate(
        "(document.querySelector('figure svg')?.outerHTML ?? '').includes('hsl(187, 28%, 26%)')",
      ),
    );
    // 暗色分支的色阶令牌同样可用
    check(
      "色阶令牌暗色可用",
      await evaluate(
        `(() => {
          const root = getComputedStyle(document.documentElement);
          const val = (v) => root.getPropertyValue(v).trim();
          const ok = ['primary','secondary','accent','muted','destructive','success']
            .every((c) => [1,2,3,4,5].every((n) => val('--' + c + '-' + n) !== ''));
          return ok && val('--primary-3') === val('--primary');
        })()`,
      ),
    );
    await shot("reader-dark-mermaid");

    // 预览徽标不遮挡工具条/标题/目录弹层（375/500/768/1024）
    for (const [width, height] of [
      [375, 812],
      [500, 900],
      [768, 900],
      [1024, 900],
    ]) {
      await send("Emulation.setDeviceMetricsOverride", {
        width,
        height,
        deviceScaleFactor: 1,
        mobile: width < 700,
      });
      await goto(`${BASE}?mode=mock&theme=light&font=17&open=preview-plan-1`);
      await waitFor(
        "document.querySelector('h1')?.textContent.includes('Python 编程')",
        10000,
      );
      const clear = await evaluate(`(() => {
        const intersects = (a, b) => !(a.right <= b.left || a.left >= b.right || a.bottom <= b.top || a.top >= b.bottom);
        const badge = document.querySelector('[data-praxis-preview-badge]');
        if (!badge) return { badge: false };
        const b = badge.getBoundingClientRect();
        const targets = [...document.querySelectorAll('.reader-toolbar, h1')].filter((el) => el.getBoundingClientRect().width > 0);
        return { badge: true, text: badge.textContent, overlap: targets.some((el) => intersects(b, el.getBoundingClientRect())) };
      })()`);
      check(
        `${width} 预览徽标不遮工具条/标题`,
        clear.badge &&
          !clear.overlap &&
          (clear.text || "").includes("刷新即复位"),
        clear,
      );
      await evaluate(
        "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '目录')?.click()",
      );
      await sleep(300);
      const tocClear = await evaluate(`(() => {
        const intersects = (a, b) => !(a.right <= b.left || a.left >= b.right || a.bottom <= b.top || a.top >= b.bottom);
        const badge = document.querySelector('[data-praxis-preview-badge]');
        const toc = document.querySelector('#reader-toc');
        if (!badge || !toc) return { badge: !!badge, toc: !!toc, overlap: true };
        return { badge: true, toc: true, overlap: intersects(badge.getBoundingClientRect(), toc.getBoundingClientRect()) };
      })()`);
      check(
        `${width} 预览徽标不遮目录弹层`,
        tocClear.badge && tocClear.toc && !tocClear.overlap,
        tocClear,
      );
      await evaluate(
        "document.querySelector('#reader-toc')?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
      );
      await sleep(200);
      await shot(`badge-${width}`);
    }

    // ---- 375 窄屏 ----
    await send("Emulation.setDeviceMetricsOverride", {
      width: 375,
      height: 812,
      deviceScaleFactor: 1,
      mobile: true,
    });
    await goto(`${BASE}?mode=mock&theme=light&font=17&open=preview-plan-1`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('Python 编程')",
      10000,
    );
    const narrow = await evaluate(`(() => {
      const main = document.querySelector('main');
      return {
        docOverflow: document.documentElement.scrollWidth > window.innerWidth,
        mainOverflow: main.scrollWidth > main.clientWidth,
        sidebar: Math.round(document.querySelector('aside').getBoundingClientRect().width),
      };
    })()`);
    check(
      "375px 不横向溢出",
      !narrow.docOverflow && !narrow.mainOverflow,
      narrow,
    );
    check("375px 侧栏收窄", narrow.sidebar === 64, narrow.sidebar);
    await shot("reader-375");

    // 375 日历：月历网格完整可见，翻页控件可达（评审确认缺陷）
    await goto(`${BASE}?mode=mock&theme=light&page=calendar`);
    await waitFor(
      "document.querySelectorAll('button[aria-label*=\"共 \"]').length > 0",
      10000,
    );
    const calNarrow = await evaluate(`(() => {
      const main = document.querySelector('main');
      const next = document.querySelector('[aria-label="下一月"]');
      const rect = next?.getBoundingClientRect();
      return {
        docOverflow: document.documentElement.scrollWidth > window.innerWidth,
        mainOverflow: main.scrollWidth > main.clientWidth,
        nextReachable: !!rect && rect.right <= window.innerWidth + 1 && rect.left >= -1,
      };
    })()`);
    check(
      "375 日历不溢出且翻页可达",
      !calNarrow.docOverflow &&
        !calNarrow.mainOverflow &&
        calNarrow.nextReachable,
      calNarrow,
    );
    await shot("calendar-375");

    // 375：七列完整、只留日期+数量、月份导航可达、年热力图局部滚动
    const cal375 = await evaluate(`(() => {
      const grid = document.querySelector('.month-grid');
      const main = document.querySelector('main');
      const cells = [...grid.querySelectorAll(':scope > button')];
      const rects = cells.map((cell) => cell.getBoundingClientRect());
      const mainRect = main.getBoundingClientRect();
      const nav = ['上一月', '下一月'].map((label) => {
        const r = document.querySelector('[aria-label="' + label + '"]')?.getBoundingClientRect();
        return !!r && r.left >= -1 && r.right <= window.innerWidth + 1;
      });
      return {
        columns: getComputedStyle(grid).gridTemplateColumns.split(' ').length,
        cells: cells.length,
        cellsInside: rects.every((r) => r.left >= mainRect.left - 1 && r.right <= mainRect.right + 1),
        chipsHidden: getComputedStyle(grid.querySelector('.month-events')).display === 'none',
        countShown: getComputedStyle(grid.querySelector('.month-count')).display !== 'none',
        navReachable: nav.every(Boolean),
        docOverflow: document.documentElement.scrollWidth > window.innerWidth,
        mainOverflow: main.scrollWidth > main.clientWidth,
      };
    })()`);
    check(
      "375 月历七列完整可见",
      cal375.columns === 7 && cal375.cells > 0 && cal375.cellsInside,
      cal375,
    );
    check(
      "375 仅日期与任务数，摘要收起",
      cal375.chipsHidden && cal375.countShown,
      cal375,
    );
    check(
      "375 月份导航可达且无页面横滚",
      cal375.navReachable && !cal375.docOverflow && !cal375.mainOverflow,
      cal375,
    );
    await evaluate(`(() => {
      const item = [...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('学习计划'));
      item?.dispatchEvent(new MouseEvent('mouseenter', { bubbles: false }));
    })()`);
    await sleep(500);
    check(
      "375 CSS 窄屏自动隐藏文字时 tooltip 可用",
      await evaluate("!!document.querySelector('.praxis-tip--show')"),
    );
    await evaluate(`(() => {
      const item = [...document.querySelectorAll('.nav-item')].find((b) => b.textContent.includes('学习计划'));
      item?.dispatchEvent(new MouseEvent('mouseleave', { bubbles: false }));
    })()`);
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '年')?.click()",
    );
    await sleep(500);
    check(
      "375 年热力图局部横滚不溢出页面",
      await evaluate(`(() => {
        const main = document.querySelector('main');
        const scroll = document.querySelector('[aria-label="近一年学习热力图"]')?.parentElement?.parentElement;
        return document.documentElement.scrollWidth <= window.innerWidth + 1 &&
          main.scrollWidth <= main.clientWidth + 1 &&
          !!scroll && getComputedStyle(scroll).overflowX === 'auto';
      })()`),
    );

    // 768 / 1024 / 1440：七列完整、恢复摘要、导航可达
    for (const [width, height] of [
      [768, 900],
      [1024, 900],
      [1440, 1000],
    ]) {
      await send("Emulation.setDeviceMetricsOverride", {
        width,
        height,
        deviceScaleFactor: 1,
        mobile: width < 700,
      });
      await goto(`${BASE}?mode=mock&theme=light&page=calendar`);
      await waitFor(
        "document.querySelectorAll('button[aria-label*=\"共 \"]').length > 0",
        10000,
      );
      const info = await evaluate(`(() => {
        const grid = document.querySelector('.month-grid');
        const main = document.querySelector('main');
        const cells = [...grid.querySelectorAll(':scope > button')];
        const last = cells[cells.length - 1]?.getBoundingClientRect();
        const mainRect = main.getBoundingClientRect();
        return {
          columns: getComputedStyle(grid).gridTemplateColumns.split(' ').length,
          inside: cells.every((cell) => {
            const r = cell.getBoundingClientRect();
            return r.left >= mainRect.left - 1 && r.right <= mainRect.right + 1;
          }),
          chipsShown: getComputedStyle(grid.querySelector('.month-events')).display !== 'none',
          navReachable: ['上一月', '下一月'].every((label) => {
            const r = document.querySelector('[aria-label="' + label + '"]')?.getBoundingClientRect();
            return !!r && r.left >= -1 && r.right <= window.innerWidth + 1;
          }),
          docOverflow: document.documentElement.scrollWidth > window.innerWidth,
          mainOverflow: main.scrollWidth > main.clientWidth,
          lastRight: last ? Math.round(last.right) : null,
        };
      })()`);
      check(
        `${width} 月历七列完整且摘要恢复`,
        info.columns === 7 &&
          info.inside &&
          info.chipsShown &&
          info.navReachable &&
          !info.docOverflow &&
          !info.mainOverflow,
        info,
      );
    }
    check(
      "日历进度面板卡片已注册（有边框/背景）",
      await evaluate(`(() => {
        const heading = [...document.querySelectorAll('h3')].find((h) => h.textContent.includes('计划进度'));
        let el = heading;
        while (el && !(el.className || '').toString().includes('bg-card')) el = el.parentElement;
        if (!el) return false;
        const style = getComputedStyle(el);
        return parseFloat(style.borderTopWidth) > 0 && style.backgroundColor !== 'rgba(0, 0, 0, 0)';
      })()`),
    );

    // ---- 日历抽屉 / 星期标注 / 设置反馈 ----
    await send("Emulation.setDeviceMetricsOverride", {
      width: 1440,
      height: 1000,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await goto(`${BASE}?mode=mock&theme=light&page=calendar`);
    await waitFor(
      "document.querySelectorAll('button[aria-label*=\"共 \"]').length > 0",
      10000,
    );
    const clickedCellLabel = await evaluate(`(() => {
      const btn = [...document.querySelectorAll('button[aria-label*="共 "]')].find((b) => !b.getAttribute('aria-label').includes('共 0 章'));
      if (!btn) return null;
      btn.click();
      return btn.getAttribute('aria-label');
    })()`);
    check(
      "日历点击打开日程抽屉",
      await waitFor("!!document.querySelector('[role=dialog]')", 4000),
    );
    const weekdayOk = await evaluate(`(() => {
      const label = ${JSON.stringify(clickedCellLabel)} ?? "";
      const m = label.match(/(\\d+) 月 (\\d+) 日/);
      if (!m) return null;
      const now = new Date();
      const date = new Date(now.getFullYear(), Number(m[1]) - 1, Number(m[2]));
      const expect = ["日", "一", "二", "三", "四", "五", "六"][date.getDay()];
      const title = document.querySelector('[role=dialog]')?.textContent ?? "";
      return title.includes("周" + expect);
    })()`);
    check("抽屉星期标注与日期一致", weekdayOk === true, {
      clickedCellLabel,
      weekdayOk,
    });
    await shot("calendar-drawer");
    await evaluate(
      "document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
    );
    await sleep(300);
    check(
      "日历抽屉可 Escape 关闭",
      await evaluate("!document.querySelector('[role=dialog]')"),
    );

    // 明确日期语义：2026-11-15 周日、18 周三、19 周四，以及跨月 12-01 周二
    const openDayTitle = async (labelPrefix) => {
      await evaluate(
        `document.querySelector('button[aria-label^="${labelPrefix}"]')?.click()`,
      );
      await waitFor("!!document.querySelector('[role=dialog]')", 3000);
      await sleep(150);
      return evaluate(
        "document.querySelector('[role=dialog]')?.textContent ?? ''",
      );
    };
    const closeDay = async () => {
      await evaluate(
        "document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
      );
      await sleep(250);
    };
    const day15 = await openDayTitle("11 月 15 日");
    check("日历星期：11-15 为周日", day15.includes("周日"), day15.slice(0, 40));
    await closeDay();
    const day18 = await openDayTitle("11 月 18 日");
    check("日历星期：11-18 为周三", day18.includes("周三"), day18.slice(0, 40));
    await closeDay();
    const day19 = await openDayTitle("11 月 19 日");
    check("日历星期：11-19 为周四", day19.includes("周四"), day19.slice(0, 40));
    await closeDay();
    await evaluate(
      "document.querySelector('[aria-label=\"下一月\"]')?.click()",
    );
    await sleep(600);
    const dayDec1 = await openDayTitle("12 月 1 日");
    check(
      "日历跨月星期：12-01 为周二",
      dayDec1.includes("周二"),
      dayDec1.slice(0, 40),
    );
    await closeDay();

    await goto(`${BASE}?mode=mock&theme=light&page=settings`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('设置')",
      8000,
    );
    await evaluate(
      "[...document.querySelectorAll('button')].find((b) => b.textContent.includes('保存并热加载'))?.click()",
    );
    await sleep(500);
    check(
      "设置保存反馈",
      await evaluate("document.body.innerText.includes('已保存并热加载')"),
    );
    await shot("settings");

    // ---- 研读页：三栏 / 直渲染 / 打字机 / 标注 / 笔记 / 聊天 ----
    await goto(`${BASE}?mode=mock&theme=light&page=study`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('研读')",
      8000,
    );
    check(
      "研读菜单与三栏结构",
      await evaluate(`(() => {
        const vis = (sel) => {
          const el = document.querySelector(sel);
          if (!el) return false;
          const r = el.getBoundingClientRect();
          return r.width > 0 && r.height > 0;
        };
        return (
          [...document.querySelectorAll('.nav-item')].some((b) => b.textContent.includes('研读')) &&
          vis('[data-study-tree]') &&
          vis('[data-study-reader]') &&
          vis('[data-study-panel]')
        );
      })()`),
    );
    check(
      "默认选最近计划",
      await evaluate(
        "(document.querySelector('[data-study-plan-select]')?.selectedOptions?.[0]?.textContent ?? '').includes('Python 编程')",
      ),
    );
    check(
      "章节树带周次徽标",
      await evaluate(
        "(document.querySelector('[data-study-tree]')?.textContent ?? '').includes('第 1 周')",
      ),
    );
    check(
      "树节点状态点：已生成章为 ready",
      await evaluate(
        "document.querySelector('[data-study-chapter=\"0:0\"] [data-status]')?.getAttribute('data-status') === 'ready'",
      ),
    );
    check(
      "选项卡带 tabpanel 关联",
      await evaluate(
        "document.querySelector('[data-study-tab=\"chat\"]')?.getAttribute('aria-controls') === 'study-tabpanel-chat' && document.querySelector('#study-tabpanel-chat')?.getAttribute('role') === 'tabpanel' && document.querySelector('#study-tabpanel-chat')?.getAttribute('aria-labelledby') === 'study-tab-chat' && document.querySelector('#study-tabpanel-note')?.getAttribute('role') === 'tabpanel' && document.querySelector('#study-tabpanel-annotations')?.getAttribute('role') === 'tabpanel'",
      ),
    );
    check(
      "选项卡方向键切换与 roving tabindex",
      await evaluate(`(async () => {
        const tab = document.querySelector('[data-study-tab="chat"]');
        tab?.focus();
        const key = (k) => tab?.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
        key('ArrowRight');
        await new Promise((resolve) => setTimeout(resolve, 80));
        const moved = document.querySelector('[data-study-tab="annotations"]')?.getAttribute('aria-selected') === 'true'
          && document.activeElement?.id === 'study-tab-annotations';
        key('Home');
        await new Promise((resolve) => setTimeout(resolve, 80));
        const back = document.querySelector('[data-study-tab="chat"]')?.getAttribute('aria-selected') === 'true'
          && document.querySelector('[data-study-tab="chat"]')?.getAttribute('tabindex') === '0'
          && document.querySelector('[data-study-tab="annotations"]')?.getAttribute('tabindex') === '-1';
        return moved && back;
      })()`),
    );
    await evaluate(`(() => {
      const chapter = document.querySelector('[data-study-chapter="0:0"]');
      chapter?.dispatchEvent(new MouseEvent('mouseenter', { bubbles: false }));
    })()`);
    await sleep(600);
    check(
      "左树章节名 tooltip 可用",
      await evaluate(
        "(document.querySelector('.praxis-tip--show')?.textContent ?? '').includes('先让程序跑起来')",
      ),
    );
    await evaluate(`(() => {
      const chapter = document.querySelector('[data-study-chapter="0:0"]');
      chapter?.dispatchEvent(new MouseEvent('mouseleave', { bubbles: false }));
    })()`);
    await sleep(300);
    await shotStudy("three-column");
    // 已生成章节：直接渲染，无打字机
    await evaluate(
      "document.querySelector('[data-study-chapter=\"0:0\"]')?.click()",
    );
    check(
      "已生成章节直接渲染正文",
      await waitFor(
        "!!document.querySelector('[data-study-content] .reader-body')",
        4000,
      ),
    );
    check(
      "直渲染无打字机残留",
      await evaluate("!document.querySelector('[data-study-typewriter]')"),
    );

    // 空章节：自动按需生成 → 打字机渐进 → 完成转 markdown
    await evaluate(`(() => {
      const select = document.querySelector('[data-study-plan-select]');
      const option = [...select.options].find((o) => o.textContent.includes('理解 Web'));
      const set = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      set.call(select, option.value);
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    check(
      "切换计划章节树更新",
      await waitFor(
        "(document.querySelector('[data-study-tree]')?.textContent ?? '').includes('待补齐的一章')",
        6000,
      ),
    );
    check(
      "树节点状态点：未研读章为 unread（空态不亮绿）",
      await evaluate(
        "[...document.querySelectorAll('[data-study-chapter]')].find((b) => b.textContent.includes('待补齐的一章'))?.querySelector('[data-status]')?.getAttribute('data-status') === 'unread'",
      ),
    );
    await shotStudy("fix-tree-status");
    await evaluate(
      "[...document.querySelectorAll('[data-study-chapter]')].find((b) => b.textContent.includes('待补齐的一章'))?.click()",
    );
    check(
      "空章自动进入打字机生成",
      await waitFor(
        "!!document.querySelector('[data-study-typewriter]')",
        4000,
      ),
    );
    const typeLen1 = await evaluate(
      "(document.querySelector('[data-study-typewriter]')?.textContent ?? '').length",
    );
    await sleep(350);
    const typeLen2 = await evaluate(
      "(document.querySelector('[data-study-typewriter]')?.textContent ?? '').length",
    );
    check("打字机文本渐进增长", typeLen2 > typeLen1, {
      typeLen1,
      typeLen2,
    });
    await shotStudy("typewriter");
    check(
      "生成完成转 markdown 全量渲染",
      await waitFor(
        "!!document.querySelector('[data-study-content] .reader-body') && (document.querySelector('[data-study-content]')?.textContent ?? '').includes('Mock 预览里按需生成') && !document.querySelector('[data-study-typewriter]')",
        12000,
      ),
    );

    // 重进已生成章节：直接渲染（不打字机）
    await evaluate(
      "document.querySelector('[data-study-chapter=\"1:0\"]')?.click()",
    );
    await sleep(400);
    await evaluate(
      "[...document.querySelectorAll('[data-study-chapter]')].find((b) => b.textContent.includes('待补齐的一章'))?.click()",
    );
    check(
      "重进已生成章节直接渲染",
      await waitFor(
        "!!document.querySelector('[data-study-content] .reader-body') && !document.querySelector('[data-study-typewriter]')",
        2500,
      ),
    );

    // 拖拽选区（受信任鼠标事件跨段拖选）：松手在阅读器外也能浮出气泡
    //（评审 P0-2：旧实现只听 section 自身 mouseup，拖出中栏松手即丢入口）
    await evaluate(
      "document.querySelector('[data-study-reader]').scrollTop = 0",
    );
    await sleep(200);
    const dragPoints = await evaluate(`(() => {
      const ps = [...document.querySelectorAll('[data-study-content] .markdown-code p')]
        .filter((p) => p.getBoundingClientRect().height > 0);
      if (ps.length < 2) return null;
      const a = ps[0].getBoundingClientRect();
      const b = ps[1].getBoundingClientRect();
      return { from: { x: Math.round(a.left + 40), y: Math.round(a.top + 6) }, to: { x: Math.round(b.left + 100), y: Math.round(b.bottom - 6) } };
    })()`);
    if (dragPoints) {
      await send("Input.dispatchMouseEvent", {
        type: "mousePressed",
        x: dragPoints.from.x,
        y: dragPoints.from.y,
        button: "left",
        buttons: 1,
        clickCount: 1,
      });
      for (let i = 1; i <= 6; i += 1) {
        await send("Input.dispatchMouseEvent", {
          type: "mouseMoved",
          x: Math.round(
            dragPoints.from.x + ((dragPoints.to.x - dragPoints.from.x) * i) / 6,
          ),
          y: Math.round(
            dragPoints.from.y + ((dragPoints.to.y - dragPoints.from.y) * i) / 6,
          ),
          button: "left",
          buttons: 1,
        });
        await sleep(40);
      }
      // 松手点落在右栏（阅读器外）：document 级监听兜住
      await send("Input.dispatchMouseEvent", {
        type: "mouseReleased",
        x: 1240,
        y: 200,
        button: "left",
        buttons: 1,
        clickCount: 1,
      });
      await sleep(400);
    }
    check(
      "拖拽跨行选区浮出标注气泡（松手在阅读器外）",
      dragPoints !== null &&
        (await evaluate("!!document.querySelector('[data-study-annotate]')")),
    );
    check(
      "标注气泡不遮挡章节头与正文标题",
      await evaluate(`(() => {
        const bubble = document.querySelector('[data-study-annotate]')?.getBoundingClientRect();
        if (!bubble) return false;
        const obstacles = [
          ...document.querySelectorAll('[data-study-reader] header, [data-study-reader] h2, [data-study-reader] h3, [data-study-reader] h4'),
        ];
        return obstacles.every((el) => {
          const r = el.getBoundingClientRect();
          return r.right <= bubble.left || r.left >= bubble.right || r.bottom <= bubble.top || r.top >= bubble.bottom;
        });
      })()`),
    );
    await shotStudy("fix-drag-bubble");
    await evaluate(`(() => {
      window.getSelection()?.removeAllRanges();
      document.querySelector('[data-study-reader]')?.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
    })()`);
    await sleep(250);

    // 标注：选中文本 → 浮动按钮 → 评论保存 → 定位 → 删除
    check(
      "选中文本浮现标注按钮",
      await evaluate(`(async () => {
        const root = document.querySelector('[data-study-content]');
        if (!root) return false;
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
        let node = walker.nextNode();
        while (node && !(node.textContent ?? '').includes('自检清单')) node = walker.nextNode();
        if (!node) return false;
        const start = (node.textContent ?? '').indexOf('自检清单');
        const range = document.createRange();
        range.setStart(node, start);
        range.setEnd(node, start + 4);
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        root.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
        // 浮动按钮由 Vue 渲染，等一帧再断言
        await new Promise((resolve) => requestAnimationFrame(resolve));
        return !!document.querySelector('[data-study-annotate]');
      })()`),
    );
    await evaluate("document.querySelector('[data-study-annotate]')?.click()");
    check(
      "标注评论浮层打开",
      await waitFor(
        "!!document.querySelector('[data-study-annotate-pop]')",
        2000,
      ),
    );
    // 点击浮层外部（正文任意处）应关闭评论浮层
    await evaluate(`(() => {
      document
        .querySelector('[data-study-content]')
        ?.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    })()`);
    check(
      "点击外部关闭标注评论浮层",
      await waitFor(
        "!document.querySelector('[data-study-annotate-pop]')",
        2000,
      ),
    );
    // 重新走一遍选区 → 按钮 → 打开浮层，继续原有保存流程
    await evaluate(`(async () => {
      const root = document.querySelector('[data-study-content]');
      if (!root) return false;
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      let node = walker.nextNode();
      while (node && !(node.textContent ?? '').includes('自检清单')) node = walker.nextNode();
      if (!node) return false;
      const start = (node.textContent ?? '').indexOf('自检清单');
      const range = document.createRange();
      range.setStart(node, start);
      range.setEnd(node, start + 4);
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      root.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
      await new Promise((resolve) => requestAnimationFrame(resolve));
      document.querySelector('[data-study-annotate]')?.click();
      return true;
    })()`);
    check(
      "标注评论浮层重新打开",
      await waitFor(
        "!!document.querySelector('[data-study-annotate-pop]')",
        2000,
      ),
    );
    await evaluate(`(() => {
      const input = document.querySelector('[data-study-annotate-pop] textarea');
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      set.call(input, '拆解思想标注');
      input.dispatchEvent(new Event('input', { bubbles: true }));
      document.querySelector('[data-study-annotate-save]')?.click();
    })()`);
    check(
      "标注保存入列表",
      await waitFor(
        "document.querySelectorAll('[data-study-annotation-item]').length === 1",
        3000,
      ),
    );
    check(
      "标注携带评论内容",
      await evaluate(
        "(document.querySelector('[data-study-annotation-item]')?.textContent ?? '').includes('拆解思想标注')",
      ),
    );
    await evaluate(
      "document.querySelector('[data-study-tab=\"annotations\"]')?.click()",
    );
    await sleep(150);
    await shotStudy("panel-annotations");
    await evaluate(
      "document.querySelector('[data-study-annotation-item] button')?.click()",
    );
    await sleep(400);
    check(
      "标注定位回原文不报错",
      await evaluate("!!document.querySelector('[data-study-content]')"),
    );
    await evaluate(
      "document.querySelector('[aria-label^=\"删除标注\"]')?.click()",
    );
    check(
      "标注删除后列表清空",
      await waitFor(
        "document.querySelectorAll('[data-study-annotation-item]').length === 0",
        3000,
      ),
    );

    // 笔记：防抖保存 → 切章回显
    await evaluate(
      "document.querySelector('[data-study-tab=\"note\"]')?.click()",
    );
    await evaluate(`(() => {
      const input = document.querySelector('[data-study-note-input]');
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      set.call(input, '# 本章笔记\\n\\n拆解：输入→处理→输出。');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    check(
      "笔记防抖保存反馈",
      await waitFor(
        "(document.querySelector('[data-study-note-status]')?.textContent ?? '').includes('已保存')",
        4000,
      ),
    );
    await shotStudy("panel-note");
    await evaluate(
      "document.querySelector('[data-study-chapter=\"1:0\"]')?.click()",
    );
    await sleep(400);
    await evaluate(
      "[...document.querySelectorAll('[data-study-chapter]')].find((b) => b.textContent.includes('待补齐的一章'))?.click()",
    );
    await sleep(700);
    check(
      "笔记重进回显",
      await evaluate(
        "document.querySelector('[data-study-note-input]')?.value.includes('拆解：输入→处理→输出')",
      ),
    );

    // 聊天：提问 → 流式 → 终态 → 历史按章独立
    await evaluate(
      "document.querySelector('[data-study-tab=\"chat\"]')?.click()",
    );
    await evaluate(`(() => {
      const input = document.querySelector('[data-study-chat-input]');
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      set.call(input, '这一章的核心方法是什么？');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await evaluate("document.querySelector('[aria-label=\"发送\"]')?.click()");
    check(
      "聊天问题即时上屏",
      await waitFor(
        "document.querySelectorAll('[data-study-chat-message]').length === 1",
        2000,
      ),
    );
    check(
      "聊天回答流式到达",
      await waitFor(
        "document.querySelector('[data-study-chat-streaming]') !== null",
        4000,
      ),
    );
    check(
      "聊天回答终态落位",
      await waitFor(
        "document.querySelectorAll('[data-study-chat-message]').length === 2 && document.querySelector('[data-study-chat-streaming]') === null",
        8000,
      ),
    );
    check(
      "回答带章节上下文话术",
      await evaluate(
        "(document.querySelectorAll('[data-study-chat-message]')[1]?.textContent ?? '').includes('输入、处理、输出')",
      ),
    );
    check(
      "用户消息气泡有色底（与助手气泡可区分）",
      await evaluate(`(() => {
        const bubbles = [...document.querySelectorAll('[data-study-chat-message] > div')];
        if (bubbles.length < 2) return false;
        const user = getComputedStyle(bubbles[0]).backgroundColor;
        const bot = getComputedStyle(bubbles[1]).backgroundColor;
        return user !== 'rgba(0, 0, 0, 0)' && user !== bot;
      })()`),
    );
    await shotStudy("panel");
    await evaluate(
      "document.querySelector('[data-study-chapter=\"1:0\"]')?.click()",
    );
    await sleep(400);
    check(
      "切换章节聊天为空（按章独立）",
      await evaluate(
        "document.querySelectorAll('[data-study-chat-message]').length === 0",
      ),
    );
    await evaluate(
      "[...document.querySelectorAll('[data-study-chapter]')].find((b) => b.textContent.includes('待补齐的一章'))?.click()",
    );
    check(
      "重进章节聊天历史回读",
      await waitFor(
        "document.querySelectorAll('[data-study-chat-message]').length === 2",
        4000,
      ),
    );
    await shotStudy("three-column-final");

    // 跨行引用定位（评审 P1-5）：引用起点跨行内代码节点（reading_minutes
    // 被拆成多个文本节点），逐字面单节点匹配必失败——规范化拼接后应命中
    await evaluate(`(() => {
      const select = document.querySelector('[data-study-plan-select]');
      const option = [...select.options].find((o) => o.textContent.includes('Python 编程'));
      const set = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      set.call(select, option.value);
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitFor(
      "(document.querySelector('[data-study-tree]')?.textContent ?? '').includes('先让程序跑起来')",
      6000,
    );
    await evaluate(
      "document.querySelector('[data-study-chapter=\"0:0\"]')?.click()",
    );
    await waitFor(
      "!!document.querySelector('[data-study-content] .reader-body')",
      4000,
    );
    await sleep(300);
    const crossQuote = await evaluate(`(() => {
      const root = document.querySelector('[data-study-content]');
      const ps = [...root.querySelectorAll('.markdown-code p')];
      const i = ps.findIndex((p) => (p.textContent ?? '').includes('reading_minutes'));
      if (i < 0 || i + 1 >= ps.length) return null;
      const startNode = ps[i].firstChild;
      const endP = ps[i + 1];
      const endNode = endP.firstChild;
      if (!startNode || !endNode || startNode === endNode) return null;
      const range = document.createRange();
      range.setStart(startNode, 1);
      range.setEnd(endNode, Math.min(10, (endNode.textContent ?? '').length));
      const quote = range.toString();
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      document.body.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
      return quote;
    })()`);
    check(
      "跨段选区浮出标注气泡（引用跨行内代码）",
      typeof crossQuote === "string" &&
        crossQuote.includes("\n") &&
        (await evaluate("!!document.querySelector('[data-study-annotate]')")),
    );
    await evaluate("document.querySelector('[data-study-annotate]')?.click()");
    await waitFor(
      "!!document.querySelector('[data-study-annotate-pop]')",
      2000,
    );
    await evaluate(`(() => {
      const input = document.querySelector('[data-study-annotate-pop] textarea');
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      set.call(input, '跨行定位验证');
      input.dispatchEvent(new Event('input', { bubbles: true }));
      document.querySelector('[data-study-annotate-save]')?.click();
    })()`);
    check(
      "跨段标注保存入列表",
      await waitFor(
        "document.querySelectorAll('[data-study-annotation-item]').length === 1",
        3000,
      ),
    );
    await evaluate(
      "document.querySelector('[data-study-tab=\"annotations\"]')?.click()",
    );
    await sleep(150);
    await evaluate(
      "document.querySelector('[data-study-reader]').scrollTop = 0",
    );
    await sleep(200);
    await evaluate(
      "document.querySelector('[data-study-annotation-item] button')?.click()",
    );
    await sleep(250);
    check(
      "跨行引用定位滚动并落在正确块",
      await evaluate(
        "document.querySelector('[data-study-reader]').scrollTop > 40 && [...document.querySelectorAll('[data-study-reader] .annotation-flash')].some((el) => (el.textContent ?? '').includes('reading_minutes'))",
      ),
    );
    await shotStudy("fix-locate-flash");
    await sleep(2000);
    check(
      "闪烁动画类按时移除",
      await evaluate(
        "!document.querySelector('[data-study-reader] .annotation-flash')",
      ),
    );
    await evaluate(
      "document.querySelector('[aria-label^=\"删除标注\"]')?.click()",
    );
    await waitFor(
      "document.querySelectorAll('[data-study-annotation-item]').length === 0",
      3000,
    );

    // 章节记忆（评审 P1-6）：按 planId 各自记忆，切换计划回到各自上次章节
    await evaluate(
      "document.querySelector('[data-study-chapter=\"1:0\"]')?.click()",
    );
    await sleep(400);
    await evaluate(`(() => {
      const select = document.querySelector('[data-study-plan-select]');
      const option = [...select.options].find((o) => o.textContent.includes('理解 Web'));
      const set = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      set.call(select, option.value);
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitFor(
      "(document.querySelector('[data-study-tree]')?.textContent ?? '').includes('待补齐的一章')",
      6000,
    );
    check(
      "章节记忆：切回计划 B 回到上次章节",
      await evaluate(
        "[...document.querySelectorAll('[data-study-chapter]')].find((b) => b.textContent.includes('待补齐的一章'))?.getAttribute('aria-current') === 'true'",
      ),
    );
    await evaluate(`(() => {
      const select = document.querySelector('[data-study-plan-select]');
      const option = [...select.options].find((o) => o.textContent.includes('Python 编程'));
      const set = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      set.call(select, option.value);
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitFor(
      "(document.querySelector('[data-study-tree]')?.textContent ?? '').includes('先让程序跑起来')",
      6000,
    );
    check(
      "章节记忆：切回计划 A 回到 1:0",
      await evaluate(
        "document.querySelector('[data-study-chapter=\"1:0\"]')?.getAttribute('aria-current') === 'true'",
      ),
    );

    // ---- 阅读设置：字号 / 行距滑块 + 持久化 ----
    await evaluate(
      "document.querySelector('[data-study-settings-btn]')?.click()",
    );
    check(
      "阅读设置：Aa 按钮打开滑块面板",
      await waitFor(
        "!!document.querySelector('[data-study-font-slider]') && !!document.querySelector('[data-study-line-slider]')",
        2000,
      ),
    );
    await evaluate(`(() => {
      const fire = (sel, val) => {
        const el = document.querySelector(sel);
        if (!el) return false;
        el.value = val;
        el.dispatchEvent(new Event('input', { bubbles: true }));
        return true;
      };
      return fire('[data-study-font-slider]', '20') && fire('[data-study-line-slider]', '2.1');
    })()`);
    await sleep(200);
    check(
      "阅读设置：字号滑块调到 20px 生效",
      await evaluate(`(() => {
        const body = document.querySelector('[data-study-reader] .reader-body');
        return !!body && parseFloat(getComputedStyle(body).fontSize) === 20;
      })()`),
    );
    check(
      "阅读设置：行距滑块调到 2.1 生效",
      await evaluate(`(() => {
        const el = document.querySelector('[data-study-reader] .markdown-code');
        if (!el) return false;
        const cs = getComputedStyle(el);
        return Math.abs(parseFloat(cs.lineHeight) / parseFloat(cs.fontSize) - 2.1) < 0.01;
      })()`),
    );
    await evaluate(
      "document.querySelector('[data-study-reader]')?.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))",
    );
    await sleep(150);
    check(
      "阅读设置：点击外部关闭面板",
      await evaluate("!document.querySelector('[data-study-reader-settings]')"),
    );

    // ---- 专注阅读：App 级顶栏/导航侧栏与本页页头两栏全部隐藏，正文独占 ----
    await evaluate(`(() => {
      const vis = (el) => { const r = el?.getBoundingClientRect(); return !!r && r.width > 0 && r.height > 0; };
      window.__focusBefore = {
        header: vis(document.querySelector('[data-study-page] header')),
        appHeader: vis(document.querySelector('.app-header')),
        sidebar: vis(document.querySelector('.app-sidebar')),
        tree: vis(document.querySelector('[data-study-tree]')),
        panel: vis(document.querySelector('[data-study-panel]')),
        readerW: document.querySelector('[data-study-reader]')?.getBoundingClientRect().width ?? 0,
      };
      return true;
    })()`);
    await evaluate("document.querySelector('[data-study-focus-btn]')?.click()");
    await sleep(300);
    check(
      "专注模式：App 顶栏/导航侧栏与本页页头两栏全部隐藏、悬浮热区就位、正文变宽",
      await evaluate(`(() => {
        const vis = (el) => { const r = el?.getBoundingClientRect(); return !!r && r.width > 0 && r.height > 0; };
        const before = window.__focusBefore ?? {};
        const readerW = document.querySelector('[data-study-reader]')?.getBoundingClientRect().width ?? 0;
        return (
          before.header === true && !vis(document.querySelector('[data-study-page] header')) &&
          before.appHeader === true && !vis(document.querySelector('.app-header')) &&
          before.sidebar === true && !vis(document.querySelector('.app-sidebar')) &&
          (!before.tree || !vis(document.querySelector('[data-study-tree]'))) &&
          (!before.panel || !vis(document.querySelector('[data-study-panel]'))) &&
          vis(document.querySelector('[data-study-focus-hotspot]')) &&
          ((before.tree || before.panel) ? readerW > before.readerW : readerW >= before.readerW)
        );
      })()`),
    );

    // Esc 直接退出专注（不再先关设置弹层），并浮现退出提示
    await evaluate(
      "document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
    );
    await sleep(400);
    check(
      "专注模式：Esc 直接退出 + 顶栏导航恢复 + 退出提示浮现",
      await evaluate(`(() => {
        const vis = (el) => { const r = el?.getBoundingClientRect(); return !!r && r.width > 0 && r.height > 0; };
        const toast = document.querySelector('[data-study-focus-toast]');
        const before = window.__focusBefore ?? {};
        const header = document.querySelector('[data-study-page] header');
        const hr = header?.getBoundingClientRect();
        return (
          !!hr && hr.height > 0 && !document.querySelector('[data-study-focus-hotspot]') &&
          vis(document.querySelector('.app-sidebar')) === (before.sidebar ?? true) &&
          !!toast && getComputedStyle(toast).opacity === '1' && (toast.textContent ?? '').includes('已退出专注模式')
        );
      })()`),
    );
    await sleep(2500);
    check(
      "专注模式：退出提示自动消失",
      await evaluate(`(() => {
        const toast = document.querySelector('[data-study-focus-toast]');
        return !!toast && parseFloat(getComputedStyle(toast).opacity) < 0.05;
      })()`),
    );

    // 悬浮工具条退出按钮：同样出提示
    await evaluate("document.querySelector('[data-study-focus-btn]')?.click()");
    await sleep(250);
    await evaluate(
      "document.querySelector('[data-study-focus-exit]')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))",
    );
    await sleep(400);
    check(
      "专注模式：悬浮工具条退出按钮同样出提示",
      await evaluate(`(() => {
        const toast = document.querySelector('[data-study-focus-toast]');
        return !!toast && getComputedStyle(toast).opacity === '1' && !document.querySelector('[data-study-focus-hotspot]');
      })()`),
    );
    await sleep(2400);

    // 刷新后阅读偏好保持（localStorage 持久化）
    await evaluate("location.reload()");
    await waitFor(
      "!!document.querySelector('[data-study-reader] .markdown-code')",
      8000,
    );
    await evaluate(
      "document.querySelector('[data-study-settings-btn]')?.click()",
    );
    await sleep(200);
    check(
      "阅读设置：刷新后滑块值保持（20px / 2.1）",
      await evaluate(`(() => {
        const fs = document.querySelector('[data-study-font-slider]');
        const ls = document.querySelector('[data-study-line-slider]');
        const el = document.querySelector('[data-study-reader] .markdown-code');
        if (!fs || !ls || !el) return false;
        const cs = getComputedStyle(el);
        return fs.value === '20' && ls.value === '2.1' &&
          parseFloat(cs.fontSize) === 20 &&
          Math.abs(parseFloat(cs.lineHeight) / parseFloat(cs.fontSize) - 2.1) < 0.01;
      })()`),
    );

    // 窄屏：单列不溢出，树抽屉收起
    await send("Emulation.setDeviceMetricsOverride", {
      width: 375,
      height: 812,
      deviceScaleFactor: 1,
      mobile: true,
    });
    await goto(`${BASE}?mode=mock&theme=light&page=study`);
    await waitFor(
      "document.querySelector('h1')?.textContent.includes('研读')",
      8000,
    );
    // 抽屉初始位移动画 200ms：等 transform 落定再量，避免量到过渡中间态
    await waitFor(
      "(() => { const r = document.querySelector('[data-study-tree]')?.getBoundingClientRect(); const m = document.querySelector('main')?.getBoundingClientRect(); return !!r && !!m && r.right <= m.left + 1; })()",
      3000,
    );
    const studyNarrow = await evaluate(`(() => {
      const main = document.querySelector('main');
      const tree = document.querySelector('[data-study-tree]');
      const tr = tree?.getBoundingClientRect();
      const mr = main?.getBoundingClientRect();
      return {
        docOverflow: document.documentElement.scrollWidth > window.innerWidth,
        mainOverflow: main.scrollWidth > main.clientWidth,
        // 窄屏主区从侧栏右侧（≈64px）开始：抽屉右沿贴住主区左沿即为完全收起
        treeOffscreen: !!tr && !!mr && tr.right <= mr.left + 1,
        toggleVisible: !!document.querySelector('[aria-label="章节目录"]'),
      };
    })()`);
    check(
      "375 研读不横向溢出且树抽屉收起",
      !studyNarrow.docOverflow &&
        !studyNarrow.mainOverflow &&
        studyNarrow.treeOffscreen &&
        studyNarrow.toggleVisible,
      studyNarrow,
    );
    await shotStudy("375");
    // 伴学抽屉不被 64px 图标侧栏裁切（评审 P0-3）：面板左沿不得越过主区
    await evaluate(
      "document.querySelector('[aria-label=\"伴学面板\"]')?.click()",
    );
    await sleep(400);
    check(
      "375 伴学面板不被图标侧栏裁切",
      await evaluate(`(() => {
        const panelAside = [...document.querySelectorAll('aside')].find((a) => a.querySelector('[data-study-panel]'));
        const sidebar = document.querySelector('.app-sidebar');
        if (!panelAside || !sidebar) return false;
        const pr = panelAside.getBoundingClientRect();
        const sr = sidebar.getBoundingClientRect();
        const title = document.querySelector('[data-study-panel] > p');
        const tr = title?.getBoundingClientRect();
        return pr.left >= sr.right - 1 &&
          pr.right <= window.innerWidth + 1 &&
          !!tr && tr.left >= sr.right - 1 && tr.right <= pr.right + 1;
      })()`),
    );
    check(
      "375 抽屉遮罩只盖中栏（不盖侧栏/页头）",
      await evaluate(`(() => {
        const overlay = [...document.querySelectorAll('main div')].find((d) => d.getAttribute('aria-hidden') === 'true' && d.className.includes('bg-foreground/25'));
        if (!overlay) return false;
        const or = overlay.getBoundingClientRect();
        const sidebar = document.querySelector('.app-sidebar')?.getBoundingClientRect();
        const main = document.querySelector('main')?.getBoundingClientRect();
        const header = document.querySelector('[data-study-page] header')?.getBoundingClientRect();
        return !!sidebar && !!main &&
          or.left >= sidebar.right - 1 &&
          or.width <= main.width + 1 &&
          !!header && or.top >= header.bottom - 1;
      })()`),
    );
    await shotStudy("fix-375-panel");
    await evaluate(
      "document.querySelector('[aria-label=\"伴学面板\"]')?.click()",
    );
    await sleep(350);

    // 1024/1280/1440 中栏不坍缩（评审 P0-1）：右栏 xl 起常驻、lg–xl 走抽屉
    for (const [width, height] of [
      [1024, 900],
      [1160, 900],
      [1280, 900],
      [1440, 1000],
    ]) {
      await send("Emulation.setDeviceMetricsOverride", {
        width,
        height,
        deviceScaleFactor: 1,
        mobile: false,
      });
      await goto(`${BASE}?mode=mock&theme=light&page=study`);
      await waitFor(
        "document.querySelector('h1')?.textContent.includes('研读')",
        8000,
      );
      const cols = await evaluate(`(() => {
        const reader = document.querySelector('[data-study-reader]');
        const tree = document.querySelector('[data-study-tree]');
        const panelAside = [...document.querySelectorAll('aside')].find((a) => a.querySelector('[data-study-panel]'));
        const toggle = document.querySelector('[aria-label="伴学面板"]');
        const chapterToggle = document.querySelector('[aria-label="章节目录"]');
        const r = reader?.getBoundingClientRect();
        const tr = tree?.getBoundingClientRect();
        const pr = panelAside?.getBoundingClientRect();
        return {
          readerWidth: r ? Math.round(r.width) : null,
          treeStatic: tr ? tr.left >= 200 : null,
          // 静态判定不能用 transform（translate-x-0 也是恒等矩阵）：
          // 看可见性 + 右缘贴合（抽屉态收起时 visibility=invisible 且整体移出屏外）
          panelStatic: pr
            ? pr.width > 200 &&
              getComputedStyle(panelAside).visibility === 'visible' &&
              Math.abs(pr.right - window.innerWidth) <= 2
            : null,
          toggleVisible:
            !!toggle &&
            toggle.getBoundingClientRect().width > 0 &&
            getComputedStyle(toggle).display !== 'none',
          chapterToggleVisible:
            !!chapterToggle &&
            chapterToggle.getBoundingClientRect().width > 0 &&
            getComputedStyle(chapterToggle).display !== 'none',
        };
      })()`);
      check(
        `${width} 研读中栏不坍缩（≥480）`,
        (cols.readerWidth ?? 0) >= 480,
        cols,
      );
      if (width === 1024 || width === 1160) {
        check(
          `${width} 右栏走抽屉（开关可见、面板不占位）`,
          cols.toggleVisible === true && cols.panelStatic === false,
          cols,
        );
        // 左树 lg 起常驻：抽屉开关必须同步隐藏，否则点击只弹全屏遮罩（无效按钮）
        check(
          `${width} 左树常驻（章节开关隐藏）`,
          cols.chapterToggleVisible === false && cols.treeStatic === true,
          cols,
        );
      }
      if (width === 1024) {
        await shotStudy("fix-1024");
      }
      if (width === 1280) {
        check(
          "1280 右栏常驻（开关隐藏、面板贴右缘）",
          cols.toggleVisible === false && cols.panelStatic === true,
          cols,
        );
      }
    }

    // 恢复视口，供后续真实数据段使用
    await send("Emulation.setDeviceMetricsOverride", {
      width: 1440,
      height: 1000,
      deviceScaleFactor: 1,
      mobile: false,
    });

    // ---- 真实数据只读 ----
    const realAvailable = await fetch(new URL("/preview-data.local.json", BASE))
      .then((r) => r.ok)
      .catch(() => false);
    if (!realAvailable) {
      console.log(
        "SKIP 真实数据模式（frontend/preview-data.local.json 不存在）",
      );
    } else {
      await goto(`${BASE}?mode=real&theme=light`);
      await waitFor(
        "document.querySelector('h1')?.textContent.includes('学习计划')",
        10000,
      );
      check(
        "真实列表有数据",
        (await evaluate(
          "document.querySelectorAll('[aria-label^=\"查看计划\"]').length",
        )) > 0,
      );
      const blocked = await evaluate(`(async () => {
        try {
          await window.__TAURI_INTERNALS__.invoke('toggle_favorite', { id: 'x' });
          return null;
        } catch (error) {
          return error.code;
        }
      })()`);
      check("真实模式拒绝写 IPC", blocked === "preview_readonly", blocked);
      const studyBoundary = await evaluate(`(async () => {
        const invoke = window.__TAURI_INTERNALS__.invoke;
        const chats = await invoke('list_chat_messages', { planId: 'x', phaseIndex: 0, chapterIndex: 0 });
        const annotations = await invoke('list_annotations', { planId: 'x', phaseIndex: 0, chapterIndex: 0 });
        const note = await invoke('get_chapter_note', { planId: 'x', phaseIndex: 0, chapterIndex: 0 });
        let writeCode = null;
        try {
          await invoke('save_chapter_note', { planId: 'x', phaseIndex: 0, chapterIndex: 0, contentMd: 'x' });
        } catch (error) {
          writeCode = error.code;
        }
        let genCode = null;
        try {
          await invoke('generate_chapter_content', { planId: 'x', phaseIndex: 0, chapterIndex: 0 });
        } catch (error) {
          genCode = error.code;
        }
        let chatCode = null;
        try {
          await invoke('chat_stream', { planId: 'x', phaseIndex: 0, chapterIndex: 0, question: 'hi' });
        } catch (error) {
          chatCode = error.code;
        }
        return { chats, annotations, note, writeCode, genCode, chatCode };
      })()`);
      check(
        "真实模式研读读命令返回空",
        Array.isArray(studyBoundary.chats) &&
          studyBoundary.chats.length === 0 &&
          Array.isArray(studyBoundary.annotations) &&
          studyBoundary.annotations.length === 0 &&
          studyBoundary.note === null,
        studyBoundary,
      );
      check(
        "真实模式研读写命令拒绝",
        studyBoundary.writeCode === "preview_readonly" &&
          studyBoundary.genCode === "preview_readonly" &&
          studyBoundary.chatCode === "preview_readonly",
        studyBoundary,
      );
      check(
        "真实模式无预览标志且正文无外链图片",
        await evaluate(
          `(() => {
            const flagAbsent = window.__PRAXIS_PREVIEW_BLOCK_REMOTE_IMAGES__ === undefined;
            const external = [...document.querySelectorAll('.reader-body img')]
              .filter((img) => /^(https?:)?\\/\\//i.test(img.getAttribute('src') ?? ''));
            return flagAbsent && external.length === 0;
          })()`,
        ),
      );
      await shot("library-real");
    }
  } finally {
    try {
      ws?.close();
    } catch {
      // ignore
    }
    chrome.kill();
    await sleep(300);
    await fs.rm(profile, { recursive: true, force: true }).catch(() => {});
  }

  check(
    "无 Vue 组件解析警告",
    !consoleWarnings.some((text) => text.includes("Failed to resolve")),
    consoleWarnings
      .filter((text) => text.includes("Failed to resolve"))
      .slice(0, 2),
  );
  const failed = results.filter((item) => !item.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 项通过`);
  if (failed.length) {
    console.error("失败项：", failed.map((item) => item.name).join("；"));
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
