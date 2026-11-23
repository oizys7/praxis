/**
 * 开发预览专用 Tauri 桥（仅由 preview.html 加载，不进入 index.html / 生产构建）。
 *
 * 两种模式：
 * - mock（默认）：内存数据，所有写操作只改内存，可完整走通生成/收藏/分组/打卡。
 * - real：加载 scripts/preview-export.py 导出的本机真实数据，只读；
 *   任何写 IPC 一律拒绝，外链图片替换为本地占位，避免自动外发请求。
 *
 * 通过 preview.html?mode=mock|real 切换；模式徽标固定于右下角。
 */

const TINY_IMAGE =
  "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI0ODAiIGhlaWdodD0iMTYwIj48cmVjdCB3aWR0aD0iNDgwIiBoZWlnaHQ9IjE2MCIgcng9IjEyIiBmaWxsPSIjZGJlN2U0Ii8+PGNpcmNsZSBjeD0iODAiIGN5PSI4MCIgcj0iMjYiIGZpbGw9IiM0ZjdmN2EiLz48cmVjdCB4PSIxNDAiIHk9IjUyIiB3aWR0aD0iMjYwIiBoZWlnaHQ9IjE0IiByeD0iNyIgZmlsbD0iIzhmYThhNCIvPjxyZWN0IHg9IjE0MCIgeT0iODIiIHdpZHRoPSIyMDAiIGhlaWdodD0iMTIiIHJ4PSI2IiBmaWxsPSIjYTliZGI5Ii8+PHJlY3QgeD0iMTQwIiB5PSIxMDYiIHdpZHRoPSIxMzAiIGhlaWdodD0iMTIiIHJ4PSI2IiBmaWxsPSIjYTliZGI5Ii8+PC9zdmc+";

const CONTENT_MAIN = [
  "## 从一个问题开始",
  "",
  "学习编程，并不只是记住语法。真正重要的是理解数据如何流动，以及怎样把一个复杂问题拆解成清晰、可验证的小步骤。今天，我们从一个简单的阅读记录程序开始，建立属于自己的知识地图。",
  "",
  "先别急着追求完整。把目标缩小到「今天能让程序跑起来」，你就已经跨过了最难的那一步。接下来每一次修改，都只是在这条能跑的路上增加一点点能力。",
  "",
  "> 不必一次掌握所有知识。先写出可以运行的最小程序，再逐步理解每一处选择。",
  "",
  "### 把大问题拆成可以验证的小问题",
  "",
  "当需求写得很大时，人会本能地停在原地。把它拆成输入、处理、输出三段，每段只回答一个问题：数据从哪里来？中间发生了什么？最后交给谁看？拆开之后，每一段都可以单独写、单独试。",
  "",
  "这个习惯会一直陪伴你：无论是写一个函数、设计一个接口，还是排查一个线上故障，先问「最小可验证的一步是什么」，再动手。",
  "",
  "## 写下第一段代码",
  "",
  "用 `reading_minutes` 记录每天的投入，观察一周的变化。",
  "",
  "```python",
  "from statistics import mean",
  "",
  "def weekly_progress(reading_minutes: list[int]) -> dict[str, float]:",
  '    """把每日阅读时长汇总成一周的阅读报告。"""',
  "    if not reading_minutes:",
  '        return {"total": 0, "average": 0}',
  "    total = sum(reading_minutes)",
  '    return {"total": total, "average": mean(reading_minutes)}',
  "",
  "# ## 代码里的假标题（不应进入本章目录）",
  "print(weekly_progress([25, 30, 20, 45, 30, 60, 40]))",
  "```",
  "",
  "### 理解输入与输出",
  "",
  "把计算和展示分开，能让代码更容易测试。尝试修改输入数组，观察输出如何变化；再试着把「平均」改成「中位数」，体会函数边界在哪里。",
  "",
  "| 概念 | 作用 | 示例 | 常见误区 | 验证方式 | 关联概念 | 备注 |",
  "| --- | --- | --- | --- | --- | --- | --- |",
  "| 输入 | 提供原始数据 | 每日阅读时长 | 把展示文本当输入 | 打印原始值 | 数据源 | 越简单越好 |",
  "| 处理 | 将数据转为信息 | 计算总时长 | 在计算里夹带输出 | 单独调用函数 | 纯函数 | 保持无副作用 |",
  "| 输出 | 呈现可理解的结果 | 一周学习报告 | 只打印不返回 | 断言返回值 | 展示层 | 可替换格式 |",
  "",
  "## 看见知识的连接",
  "",
  "代码只是结果，结构才是可以迁移的部分。下面这张图把一次「收集 → 计算 → 展示」的流程画了出来。",
  "",
  "```mermaid",
  "flowchart LR",
  "  A[收集数据] --> B[处理与计算]",
  "  B --> C[输出学习报告]",
  "  B --> D[保存历史]",
  "  D --> C",
  "```",
  "",
  "图里的每个节点都可以继续展开。阅读时不妨停一下，问自己：如果数据源换成数据库，哪一段需要改？",
  "",
  "## 图片与长内容",
  "",
  "合适的示意图能让抽象关系变得可见：",
  "",
  "![阅读数据示意](" + TINY_IMAGE + ")",
  "",
  "下面这张图故意指向不存在的地址，用来验证图片失败时的占位提示，而不是留下一枚裂图：",
  "",
  "![缺失的示意图](/preview-missing-image.png)",
  "",
  "![外链图片测试](https://images.example.invalid/review-image.png)",
  "",
  '<picture><source srcset="https://images.example.invalid/review-srcset.png 1x"><img src="https://images.example.invalid/review-img.png" alt="外链 picture 测试"></picture>',
  "",
  "## 动手练习",
  "",
  "1. 为程序添加一个「最长单次阅读」字段。",
  "2. 检查空数组时会发生什么，并写一个测试。",
  "3. 用自己的话解释函数的作用，再让同伴复述一遍。",
  "",
  "### 一个更长的代码块",
  "",
  "当一行很长时，代码块应该在本地区域横向滚动，而不是把整页撑宽。下面这行刻意写得很长：",
  "",
  "```javascript",
  'const report = { total: 250, average: 35.7, streak: 7, note: "把每天的最小行动固定下来，比偶尔爆发更重要；当状态不好时，只完成最小闭环，也算保持连续性。" };',
  "```",
  "",
  "未知语言的代码块不应该报错，只按纯文本高亮：",
  "",
  "```brainfuck",
  "++++++++[>++++[>++>+++>+++>+<<<<-]>+>+>->>+[<]<-]>>.>---.+++++++..+++.",
  "```",
  "",
  "### 失败图示的降级",
  "",
  "```mermaid",
  "flowchart LR",
  "  A --> --> B",
  "```",
  "",
  "上面是语法错误的图示，应当降级为源码，而不是让整章白屏。",
  "",
  "### 转义括号的自动修复",
  "",
  "模型偶尔会在节点文本里用反斜杠转义括号（词法不认），应当引号化修复后正常成图：",
  "",
  "```mermaid",
  "flowchart LR",
  "  A[需要新线程] --> B{需要返回值吗?}",
  "  B -- 否 --> C[继承 Thread]",
  "  B -- 是 --> D[Callable + FutureTask]",
  "  C --> E[t.start\\(\\)]",
  "  D --> E",
  "  E --> F[新线程执行 run/call]",
  "```",
  "",
  "## 安全边界自检",
  "",
  "正文来自模型，必须经过消毒。下面这些危险片段不应执行：",
  "",
  '<img src="x" alt="危险图片" onerror="window.__praxisXss = 1">',
  "",
  "[危险链接](javascript:window.__praxisXss = 2)",
  "",
  "## 收束",
  "",
  "读到这里，你已经完成了一次从问题到代码、再到可视化表达的小循环。下一章我们会把这份记录器变成一个可以持续使用的小项目：加入存储、加入回顾，也加入对自己的观察。",
  "",
  "学习不是把知识搬进脑子，而是不断用更小的反馈修正自己的理解。慢慢来，但不要停。",
].join("\n");

const CONTENT_SHORT = [
  "## 本章要点",
  "",
  "这一章用更短的篇幅复习上一章的结构：先明确输入，再写下处理，最后决定如何展示。",
  "",
  "1. 输入尽量保持原始，不要提前格式化。",
  "2. 处理保持纯净，便于单独验证。",
  "3. 输出只负责表达，随时可以替换。",
  "",
  "> 短章节同样值得认真读：它把长章节里散落的线索收成一句可以带走的话。",
  "",
  "```sql",
  "SELECT plan_id, COUNT(*) AS chapters",
  "FROM chapters GROUP BY plan_id ORDER BY chapters DESC;",
  "```",
].join("\n");

// ---------------------------------------------------------------------------
// 通用桥
// ---------------------------------------------------------------------------

function createBridge({
  mode,
  records,
  groups,
  calendar,
  readOnly,
  settings,
  onWrite,
}) {
  const callbacks = new Map();
  const listeners = new Map();
  const handlerEvents = new Map();
  let nextCallbackId = 1;
  let nextEventId = 1;

  window.__previewRecords = records;
  window.__previewGroups = groups;
  window.__previewCalls = [];
  window.__previewMode = mode;

  function emitEvent(name, payload) {
    for (const handlerId of listeners.get(name) ?? []) {
      const handler = callbacks.get(handlerId);
      if (handler) {
        handler({ event: name, id: nextEventId++, payload });
      }
    }
  }

  function clone(value) {
    return JSON.parse(JSON.stringify(value));
  }

  function readonlyError(cmd) {
    const error = {
      code: "preview_readonly",
      message: `真实数据预览为只读模式，已拒绝执行「${cmd}」`,
      hint: "写操作请在 Mock 模式验证；真实数据库不会被修改。",
    };
    throw error;
  }

  async function invoke(cmd, args = {}) {
    window.__previewCalls.push({
      cmd,
      args: args && typeof args === "object" ? { ...args } : args,
    });

    if (cmd === "startup_status") {
      return {
        ready: true,
        phase: "ready",
        message: "预览已就绪",
        progress: 100,
        degraded: null,
        elapsedMs: 12,
      };
    }
    if (cmd === "ping_agent") return true;

    // 事件插件：注册回调 id，卸载按 id 清理
    if (cmd === "plugin:event|listen") {
      const eventId = nextEventId++;
      handlerEvents.set(eventId, { event: args.event, handler: args.handler });
      const set = listeners.get(args.event) ?? new Set();
      set.add(args.handler);
      listeners.set(args.event, set);
      return eventId;
    }
    if (cmd === "plugin:event|unlisten") {
      const entry = handlerEvents.get(args.eventId);
      if (entry) {
        listeners.get(entry.event)?.delete(entry.handler);
        handlerEvents.delete(args.eventId);
      }
      return;
    }
    if (cmd === "plugin:event|emit" || cmd === "plugin:event|emit_to") {
      emitEvent(args.event, args.payload);
      return;
    }

    if (readOnly) {
      if (cmd === "list_plans") {
        const query = String(args.query ?? "")
          .trim()
          .toLowerCase();
        return clone(
          records
            .filter(
              (r) =>
                !query || `${r.title} ${r.goal}`.toLowerCase().includes(query),
            )
            .map(toSummary),
        );
      }
      if (cmd === "list_plan_groups") return clone(groups);
      if (cmd === "get_plan") {
        const record = records.find((r) => r.id === args.id);
        if (!record) {
          throw {
            code: "preview_not_exported",
            message: "该计划未包含在本次只读导出中",
            hint: "导出脚本默认只包含最近几份完整计划；调整 PRAXIS_PREVIEW_PLANS 后可重新导出。",
          };
        }
        return clone(record);
      }
      if (cmd === "get_settings") return clone(settings);
      if (cmd === "query_month_calendar") {
        const days = (calendar.month ?? []).filter((day) => {
          const [y, m] = String(day.date).split("-").map(Number);
          return y === Number(args.year) && m === Number(args.month);
        });
        return clone(days);
      }
      if (cmd === "query_year_heatmap") {
        return clone(
          (calendar.year ?? []).filter((day) =>
            String(day.date).startsWith(`${args.year}-`),
          ),
        );
      }
      return readonlyError(cmd);
    }

    return mockCommand(cmd, args);
  }

  function mockCommand(cmd, args) {
    if (cmd === "list_plans") {
      const query = String(args.query ?? "")
        .trim()
        .toLowerCase();
      return records
        .filter(
          (r) => !query || `${r.title} ${r.goal}`.toLowerCase().includes(query),
        )
        .map(toSummary)
        .sort(
          (a, b) =>
            Number(b.favorite) - Number(a.favorite) ||
            String(b.createdAt).localeCompare(String(a.createdAt)),
        );
    }
    if (cmd === "list_plan_groups") {
      return groups.map((group) => ({
        ...group,
        planCount: records.filter((record) => record.groupId === group.id)
          .length,
      }));
    }
    if (cmd === "get_plan") {
      const record = records.find((r) => r.id === args.id);
      if (!record) {
        throw { code: "not_found", message: "预览数据中找不到这份计划" };
      }
      return clone(record);
    }
    if (cmd === "get_settings") return { ...settings };
    if (cmd === "save_settings") {
      Object.assign(settings, args.settings ?? {});
      return;
    }
    if (cmd === "toggle_favorite") {
      const record = records.find((r) => r.id === args.id);
      if (!record) return false;
      record.favorite = !record.favorite;
      return record.favorite;
    }
    if (cmd === "create_plan_group") {
      const group = {
        id: `preview-group-${groups.length + 1}`,
        name: String(args.name ?? "").trim(),
        planCount: 0,
      };
      groups.push(group);
      return clone(group);
    }
    if (cmd === "rename_plan_group") {
      const group = groups.find((g) => g.id === args.id);
      if (group) group.name = String(args.name ?? "").trim();
      return;
    }
    if (cmd === "delete_plan_group") {
      const index = groups.findIndex((g) => g.id === args.id);
      if (index >= 0) groups.splice(index, 1);
      records.forEach((r) => {
        if (r.groupId === args.id) r.groupId = null;
      });
      return;
    }
    if (cmd === "set_plan_group") {
      const record = records.find((r) => r.id === args.id);
      if (record) record.groupId = args.groupId ?? null;
      return;
    }
    if (cmd === "delete_plan") {
      const index = records.findIndex((r) => r.id === args.id);
      if (index >= 0) records.splice(index, 1);
      return;
    }
    if (cmd === "start_learning") {
      const record = records.find((r) => r.id === args.id);
      if (record) {
        record.learningStatus = "active";
        record.learningStartedAt = todayKey();
        if (Array.isArray(args.days) && args.days.length)
          record.studyDays = [...args.days];
      }
      return;
    }
    if (cmd === "update_learning_days") {
      const record = records.find((r) => r.id === args.id);
      if (record && Array.isArray(args.days)) record.studyDays = [...args.days];
      return;
    }
    if (cmd === "toggle_schedule_item") {
      const key = `${args.id}:${args.date}:${args.phaseIdx}:${args.chapterIdx}`;
      scheduleState.set(key, Boolean(args.done));
      return;
    }
    if (cmd === "query_month_calendar")
      return buildMonth(Number(args.year), Number(args.month));
    if (cmd === "query_year_heatmap") return buildYear(Number(args.year));
    if (cmd === "generate_plan") {
      // 预览失败场景：?gen=fail 用于验证表单/输入保留与错误反馈
      if (new URLSearchParams(location.search).get("gen") === "fail") {
        throw {
          code: "preview_generation_failed",
          message: "模拟生成失败：预览未调用模型",
          hint: "这是 Mock 失败场景，表单与已填内容应保留",
        };
      }
      return startGeneration(args.request);
    }
    if (cmd === "regenerate_plan") {
      const record = records.find((r) => r.id === args.planId);
      if (!record)
        throw { code: "not_found", message: "预览数据中找不到这份计划" };
      resetForGeneration(record);
      simulateGeneration(record, 220);
      return { planId: record.id };
    }
    if (cmd === "regenerate_phase") {
      const record = records.find((r) => r.id === args.planId);
      if (!record)
        throw { code: "not_found", message: "预览数据中找不到这份计划" };
      const phase = record.plan.phases[args.phaseIndex];
      if (phase) {
        phase.status = "complete";
        phase.chapters = (
          phase.chapters?.length ? phase.chapters : makeChapters(phase.name)
        ).map((chapter) => ({
          ...chapter,
          status: "complete",
          contentMd: markRegenerated(chapter.contentMd),
        }));
      }
      return planEnvelope(record);
    }
    if (cmd === "regenerate_chapter") {
      const record = records.find((r) => r.id === args.planId);
      if (!record)
        throw { code: "not_found", message: "预览数据中找不到这份计划" };
      const chapter =
        record.plan.phases[args.phaseIndex]?.chapters?.[args.chapterIndex];
      if (chapter) {
        chapter.status = "complete";
        chapter.contentMd = markRegenerated(chapter.contentMd);
      }
      return planEnvelope(record);
    }
    if (cmd === "plugin:event|unlisten") return;

    throw { code: "preview", message: `预览尚未模拟此操作：${cmd}` };
  }

  // ---------------- mock 动态行为 ----------------

  const scheduleState = new Map();

  function planEnvelope(record) {
    return {
      status: "plan",
      plan: clone(record.plan),
      clarifications: null,
      error: null,
    };
  }

  function markRegenerated(contentMd) {
    const notice = "> 本章已在本机 Mock 预览中重新生成。\n\n";
    return notice + (contentMd || CONTENT_SHORT);
  }

  function resetForGeneration(record) {
    record.status = "generating";
    record.plan.phases = record.plan.phases.map((phase) => ({
      ...phase,
      status: "pending",
      chapters: (phase.chapters ?? []).map((chapter) => ({
        ...chapter,
        status: "pending",
      })),
    }));
  }

  function startGeneration(request) {
    const goal = String(request?.goal ?? "新的学习目标");
    const id = `preview-generated-${Date.now()}`;
    const record = makeRecord({
      id,
      title: goal.length > 22 ? `${goal.slice(0, 22)}…` : goal,
      goal,
      level: request?.level ?? "beginner",
      weeks: request?.weeks ?? 8,
      overview:
        "这是一份在 Mock 预览中即时生成的计划：先建立最小可验证的能力，再用一个小项目把知识串起来。",
      status: "generating",
      favorite: false,
      groupId: null,
      learningStatus: "not_started",
      learningStartedAt: null,
      studyDays: request?.studyDays ?? [1, 2, 3, 4, 5, 6, 7],
      createdAt: new Date().toISOString(),
    });
    records.unshift(record);
    resetForGeneration(record);
    simulateGeneration(record, 420);
    return { planId: id };
  }

  function simulateGeneration(record, firstDelay) {
    const phases = record.plan.phases;
    window.setTimeout(() => {
      emitEvent("plan-progress", {
        planId: record.id,
        kind: "plan",
        payload: {
          type: "outline",
          phaseCount: phases.length,
          title: record.plan.title,
          overview: record.plan.overview,
          phases: phases.map((phase) => ({
            name: phase.name,
            weekRange: phase.weekRange,
            chapterTitles: (phase.chapters ?? []).map(
              (chapter) => chapter.title,
            ),
          })),
        },
      });
    }, firstDelay);

    let elapsed = firstDelay;
    phases.forEach((phase, phaseIndex) => {
      (phase.chapters ?? []).forEach((_, chapterIndex) => {
        elapsed += 260;
        window.setTimeout(() => {
          emitEvent("plan-progress", {
            planId: record.id,
            kind: "plan",
            payload: { type: "chapterStart", phaseIndex, chapterIndex },
          });
        }, elapsed);
        elapsed += 180;
        window.setTimeout(() => {
          emitEvent("plan-progress", {
            planId: record.id,
            kind: "plan",
            payload: { type: "chapterDone", phaseIndex, chapterIndex },
          });
        }, elapsed);
      });
      elapsed += 240;
      window.setTimeout(() => {
        phase.status = "complete";
        (phase.chapters ?? []).forEach((chapter) => {
          chapter.status = "complete";
          if (!chapter.contentMd) chapter.contentMd = CONTENT_SHORT;
        });
        emitEvent("plan-progress", {
          planId: record.id,
          kind: "plan",
          payload: {
            type: "phaseDone",
            index: phaseIndex,
            total: phases.length,
          },
        });
        if (phaseIndex === phases.length - 1) {
          record.status = "complete";
        }
      }, elapsed);
    });
  }

  function buildMonth(year, month) {
    const plan = records.find((r) => r.plan?.phases?.length);
    if (!plan || !Number.isFinite(year) || !Number.isFinite(month)) return [];
    const chapters = [];
    plan.plan.phases.forEach((phase, phaseIndex) => {
      (phase.chapters ?? []).forEach((chapter, chapterIndex) => {
        chapters.push({ phase, phaseIndex, chapter, chapterIndex });
      });
    });
    const days = [];
    chapters.slice(0, 10).forEach((entry, index) => {
      const day = 3 + index * 3;
      if (day > 27) return;
      const date = `${year}-${pad2(month)}-${pad2(day)}`;
      const key = `${plan.id}:${date}:${entry.phaseIndex}:${entry.chapterIndex}`;
      const done = scheduleState.has(key) ? scheduleState.get(key) : index < 2;
      days.push({
        date,
        items: [
          {
            planId: plan.id,
            planTitle: plan.title,
            phaseIdx: entry.phaseIndex,
            phaseName: entry.phase.name,
            chapterIdx: entry.chapterIndex,
            chapterTitle: entry.chapter.title,
            done,
          },
        ],
        total: 1,
        doneCount: done ? 1 : 0,
      });
    });
    return days;
  }

  function buildYear(year) {
    if (!Number.isFinite(year)) return [];
    const out = [];
    for (let month = 1; month <= 12; month += 1) {
      const days = buildMonth(year, month).slice(0, 4);
      days.forEach((day) => {
        out.push({ date: day.date, total: day.total, done: day.doneCount });
      });
    }
    return out;
  }

  window.__TAURI_INTERNALS__ = {
    transformCallback(callback) {
      const id = nextCallbackId++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id) {
      callbacks.delete(id);
    },
    invoke,
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, eventId) {
      const entry = handlerEvents.get(eventId);
      if (entry) {
        listeners.get(event)?.delete(entry.handler);
        handlerEvents.delete(eventId);
      }
    },
  };

  // 调试口：Mock 模式下可手动广播事件（评审脚本/控制台均可复现生成动画）
  window.__previewEmit = (event, payload) => emitEvent(event, payload);
}

function toSummary(record) {
  const chapters =
    record.plan?.phases?.flatMap((phase) => phase.chapters ?? []) ?? [];
  const total = chapters.length;
  const done = chapters.filter(
    (chapter) => chapter.status === "complete",
  ).length;
  return {
    id: record.id,
    title: record.title,
    goal: record.goal,
    status: record.status,
    overview: record.overview,
    level: record.level,
    weeks: record.weeks,
    groupId: record.groupId ?? null,
    studyDays: record.studyDays ?? [1, 2, 3, 4, 5, 6, 7],
    learningStatus: record.learningStatus ?? "not_started",
    learningStartedAt: record.learningStartedAt ?? null,
    chapterTotal: total,
    chapterDone: done,
    studyTotal: total,
    studyDone: record.learningStatus === "active" ? Math.min(2, done) : 0,
    favorite: Boolean(record.favorite),
    createdAt: record.createdAt,
  };
}

function pad2(value) {
  return String(value).padStart(2, "0");
}

function todayKey() {
  const now = new Date();
  return `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())}`;
}

function makeChapters(prefix) {
  return [
    {
      title: `${prefix}：先让程序跑起来`,
      summary: "从最小可验证的一步开始，把问题拆成输入、处理与输出。",
      contentMd: CONTENT_MAIN,
      kind: "concept",
      keyTerms: ["输入", "处理", "输出"],
      visualizations: [],
      exercises: [],
      done: true,
      status: "complete",
    },
    {
      title: `${prefix}：把知识变成作品`,
      summary: "用一个小练习巩固理解，并记录过程中遇到的阻碍。",
      contentMd: CONTENT_SHORT,
      kind: "practice",
      keyTerms: ["反馈", "复盘"],
      visualizations: [],
      exercises: [],
      done: false,
      status: "complete",
    },
  ];
}

function makeProject(name) {
  return {
    name,
    description: "用一个小项目把本阶段的知识串起来，形成可以展示的成果。",
    background: "真实的问题比练习题更能暴露理解上的缺口。",
    steps: ["列出最小功能", "实现并运行", "写下一段复盘"],
    estimatedHours: 6,
    acceptanceCriteria: [
      "程序可以运行",
      "覆盖一个边界情况",
      "能用自己的话解释实现",
    ],
  };
}

function makePhases(names, { withEmptyChapter = false } = {}) {
  return names.map((name, index) => ({
    name,
    weekRange: `第 ${index * 2 + 1}–${index * 2 + 2} 周`,
    objectives: [
      "理解核心概念，并能用自己的话解释",
      "通过小项目建立可迁移的学习方法",
    ],
    chapters:
      withEmptyChapter && index === 1
        ? [
            ...makeChapters(name),
            {
              title: `${name}：待补齐的一章`,
              summary: "这一章保留为空正文，用于验证空章提示与重新生成入口。",
              contentMd: "",
              kind: "concept",
              keyTerms: [],
              visualizations: [],
              exercises: [],
              done: false,
              status: "pending",
            },
          ]
        : makeChapters(name),
    theory: [],
    practice: ["完成本章的动手练习"],
    assignments: ["提交一段可以运行的小程序"],
    project: makeProject(`${name} · 收尾项目`),
    status: "complete",
  }));
}

function makeRecord(input) {
  const title = input.title;
  return {
    id: input.id,
    title,
    goal: input.goal,
    status: input.status ?? "complete",
    overview: input.overview ?? "",
    level: input.level ?? "beginner",
    weeks: input.weeks ?? 8,
    studyDays: input.studyDays ?? [1, 3, 5],
    favorite: Boolean(input.favorite),
    groupId: input.groupId ?? null,
    learningStatus: input.learningStatus ?? "not_started",
    learningStartedAt: input.learningStartedAt ?? null,
    createdAt: input.createdAt,
    plan: {
      title,
      overview: input.overview ?? "",
      estimatedTotalHours: input.estimatedTotalHours ?? 80,
      phases:
        input.phases ??
        makePhases(["建立编程思维", "探索数据与结构", "完成独立项目"]),
      resources: input.resources ?? [
        {
          title: "Python 官方教程",
          type: "docs",
          note: "从语言结构开始读，配合动手练习。",
          url: "https://docs.python.org/zh-cn/3/tutorial/",
          verification: "unverified",
        },
      ],
      reviewNotes:
        input.reviewNotes ?? "按自己的节奏推进，每个阶段结束后回顾一次。",
      glossary: input.glossary ?? [
        { term: "输入", chapterTitle: "建立编程思维：先让程序跑起来" },
        { term: "纯函数", chapterTitle: "建立编程思维：先让程序跑起来" },
      ],
    },
  };
}

function buildMockRecords() {
  const longTitle =
    "从零开始构建一个可持续维护的个人知识管理系统：阅读、记录、回顾与输出的完整闭环";
  return [
    makeRecord({
      id: "preview-plan-1",
      title: "Python 编程：从理解到独立实践",
      goal: "用 Python 解决实际问题，完成第一个可以展示的独立项目",
      level: "beginner",
      weeks: 8,
      favorite: true,
      groupId: "preview-group-1",
      learningStatus: "active",
      learningStartedAt: "2026-11-02",
      studyDays: [1, 3, 5],
      createdAt: "2026-11-18T10:00:00Z",
      overview:
        "从第一行代码出发，逐步掌握编程思维、数据处理和项目实践。把每一个新概念放进具体问题中，在持续阅读与动手之间建立自己的知识体系。",
    }),
    makeRecord({
      id: "preview-plan-2",
      title: "理解 Web：从浏览器到服务端",
      goal: "理解一次网络请求与页面呈现的完整过程",
      level: "intermediate",
      weeks: 6,
      status: "partial",
      groupId: null,
      learningStatus: "not_started",
      createdAt: "2026-11-16T10:00:00Z",
      overview:
        "沿着一次点击发生的过程，理解浏览器、HTTP、服务端与数据存储如何协作。部分章节保留待生成状态，用于验证补齐流程。",
      phases: makePhases(["请求是怎么发生的", "数据如何抵达页面"], {
        withEmptyChapter: true,
      }),
    }),
    makeRecord({
      id: "preview-plan-3",
      title: "设计的日常：视觉与信息秩序",
      goal: "培养清晰、克制的设计判断，并能解释自己的选择",
      level: "beginner",
      weeks: 4,
      status: "failed",
      groupId: null,
      learningStatus: "not_started",
      createdAt: "2026-11-15T10:00:00Z",
      overview:
        "这份计划在生成中途失败，保留了部分章节，用于验证失败提示与重新生成入口。",
      phases: [
        {
          ...makePhases(["建立视觉判断"])[0],
          status: "failed",
          chapters: [
            {
              title: "密度与留白：让信息有呼吸",
              summary: "从间距、层级与对齐开始，建立可解释的视觉判断。",
              contentMd: "",
              kind: "concept",
              keyTerms: [],
              visualizations: [],
              exercises: [],
              done: false,
              status: "failed",
            },
          ],
        },
      ],
    }),
    makeRecord({
      id: "preview-plan-4",
      title: longTitle,
      goal: "搭建一个可以长期使用的个人知识管理系统，把输入转化为可复用的输出",
      level: "advanced",
      weeks: 12,
      favorite: false,
      groupId: "preview-group-2",
      learningStatus: "not_started",
      createdAt: "2026-11-12T10:00:00Z",
      overview:
        "从信息收集、结构整理到回顾与输出，建立一套低摩擦、可持续的个人知识工作流。",
      phases: makePhases(["收集与整理", "连接与回顾", "输出与迭代"]),
    }),
  ];
}

function buildMockGroups() {
  return [
    { id: "preview-group-1", name: "编程基础", planCount: 1 },
    { id: "preview-group-2", name: "设计与前端", planCount: 1 },
    { id: "preview-group-empty", name: "待读清单", planCount: 0 },
  ];
}

function safeSettings(allowMockLlm = true) {
  return {
    llmProvider: "openai",
    openaiApiKey: "",
    openaiModel: "gpt-4o-mini",
    anthropicApiKey: "",
    anthropicModel: "claude-3-5-sonnet-latest",
    deepseekApiKey: "",
    deepseekModel: "deepseek-chat",
    glmApiKey: "",
    glmModel: "glm-5.3-flash",
    opencodeApiKey: "",
    opencodeModel: "grok-4.7",
    allowMockLlm,
  };
}

function mountPreviewBadge(mode) {
  if (document.querySelector("[data-praxis-preview-badge]")) return;
  const badge = document.createElement("div");
  badge.dataset.praxisPreviewBadge = "true";
  // 固定左下角：顶部 84px 会压住阅读工具条，右下会与目录浮层/回顶按钮相撞
  badge.style.cssText = [
    "position:fixed",
    "left:12px",
    "bottom:12px",
    "max-width:calc(100vw - 24px)",
    "z-index:9998",
    "display:flex",
    "align-items:center",
    "gap:8px",
    "white-space:nowrap",
    "padding:7px 11px",
    "border-radius:999px",
    "border:1px solid hsl(var(--border))",
    "background:hsl(var(--popover) / 0.94)",
    "color:hsl(var(--popover-foreground))",
    "font:12px/1.5 'Segoe UI','PingFang SC','Microsoft YaHei',sans-serif",
    "box-shadow:0 6px 24px rgb(20 35 50 / 0.16)",
    "backdrop-filter:blur(8px)",
    "pointer-events:auto",
  ].join(";");
  const label = document.createElement("span");
  label.textContent =
    mode === "real" ? "预览 · 真实数据只读" : "预览 · Mock 内存数据，刷新即复位";
  label.title =
    mode === "real"
      ? "来自本机数据库的只读导出，不上传；所有写操作被拒绝"
      : "内存数据模拟，写操作只影响当前页面；界面里的「保存到云端」在预览中并非真实写入";
  const link = document.createElement("a");
  link.href = mode === "real" ? "?mode=mock" : "?mode=real";
  link.textContent = mode === "real" ? "切到 Mock" : "查看真实数据";
  link.style.cssText =
    "color:hsl(var(--primary));text-decoration:underline;text-underline-offset:3px;white-space:nowrap";
  badge.append(label, link);
  document.body.append(badge);
}

/** Mock 模式：内存数据 + 完整写操作模拟 */
export function installMockBridge() {
  window.__PRAXIS_PREVIEW_MODE = "mock";
  createBridge({
    mode: "mock",
    records: buildMockRecords(),
    groups: buildMockGroups(),
    calendar: { month: [], year: [] },
    readOnly: false,
    settings: safeSettings(true),
  });
  mountPreviewBadge("mock");
}

/**
 * 真实数据只读预览不外发请求：把正文里的外链图片改写为占位引用，避免把
 * 内容中的 URL/IP 泄给第三方。在数据进入渲染管线之前完成，生产组件不含
 * 任何预览专用分支；数据 URL 与相对路径不受影响（后者失败走图片失败占位）。
 */
function rewriteExternalImages(records) {
  const pattern =
    /!\[([^\]]*)\]\(\s*(?:https?:\/\/|\/\/)[^)\s]+\s*(?:"[^"]*")?\s*\)/g;
  for (const record of records) {
    for (const phase of record?.plan?.phases ?? []) {
      for (const chapter of phase?.chapters ?? []) {
        if (typeof chapter?.contentMd === "string" && chapter.contentMd.includes("](")) {
          chapter.contentMd = chapter.contentMd.replace(
            pattern,
            (_match, alt) =>
              `> ${
                alt
                  ? `外部图片已本地屏蔽：${alt}`
                  : "外部图片已本地屏蔽（真实数据预览不外发请求）"
              }\n`,
          );
        }
      }
    }
  }
}

/** 真实模式：只读导出数据；所有写 IPC 拒绝；外链图片在数据入口本地化 */
export function installRealBridge(data) {
  if (!data || !Array.isArray(data.plans) || typeof data.records !== "object") {
    throw new Error("预览数据格式不完整");
  }
  window.__PRAXIS_PREVIEW_MODE = "real";
  const records = Object.values(data.records);
  rewriteExternalImages(records);
  createBridge({
    mode: "real",
    records,
    groups: Array.isArray(data.groups) ? data.groups : [],
    calendar: data.calendar ?? { month: [], year: [] },
    readOnly: true,
    settings: safeSettings(false),
  });
  // 列表摘要与完整记录分开：只读模式 list_plans 直接用导出摘要，
  // 避免为未导出计划的摘要去重造详情。
  const internals = window.__TAURI_INTERNALS__;
  const baseInvoke = internals.invoke;
  internals.invoke = async (cmd, args = {}) => {
    if (cmd === "list_plans") {
      window.__previewCalls.push({ cmd, args });
      const query = String(args.query ?? "")
        .trim()
        .toLowerCase();
      return data.plans.filter(
        (plan) =>
          !query || `${plan.title} ${plan.goal}`.toLowerCase().includes(query),
      );
    }
    return baseInvoke(cmd, args);
  };
  mountPreviewBadge("real");
}

export function showPreviewError(error) {
  const box = document.createElement("div");
  box.style.cssText = [
    "position:fixed",
    "inset:0",
    "z-index:2147483000",
    "display:flex",
    "align-items:center",
    "justify-content:center",
    "padding:24px",
    "background:hsl(210 20% 97%)",
  ].join(";");
  const card = document.createElement("div");
  card.style.cssText = [
    "max-width:560px",
    "padding:28px 32px",
    "border:1px solid hsl(208 18% 87%)",
    "border-radius:16px",
    "background:#fff",
    "color:hsl(214 27% 21%)",
    "font:14px/1.9 'Segoe UI','PingFang SC','Microsoft YaHei',sans-serif",
  ].join(";");
  const title = document.createElement("h1");
  title.textContent = "真实数据预览不可用";
  title.style.cssText = "font-size:18px;font-weight:600;margin:0 0 10px";
  const body = document.createElement("p");
  body.textContent = String(error?.message ?? error ?? "未知错误");
  body.style.cssText = "margin:0 0 14px;color:hsl(212 12% 43%)";
  const steps = document.createElement("pre");
  steps.textContent =
    'cd agent-service && uv pip install "psycopg[binary]"\ncd .. && agent-service/.venv/Scripts/python scripts/preview-export.py\n然后刷新本页（?mode=real）';
  steps.style.cssText =
    "margin:0;padding:12px 14px;border-radius:10px;background:hsl(207 18% 94%);white-space:pre-wrap;font-size:12px;line-height:1.8";
  card.append(title, body, steps);
  box.append(card);
  document.body.append(box);
}
