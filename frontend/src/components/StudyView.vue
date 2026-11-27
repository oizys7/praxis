<script setup lang="ts">
import {
  computed,
  inject,
  nextTick,
  onMounted,
  onUnmounted,
  ref,
  watch,
} from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  BookOpenText,
  CheckCircle2,
  Focus,
  MessagesSquare,
  TriangleAlert,
  Type,
} from "lucide-vue-next";
import {
  isRegenInFlight,
  normalizeError,
  regenScopes,
} from "@/composables/useAgent";
import {
  fetchPlanRecord,
  fetchPlanSummaries,
} from "@/composables/usePlanHistory";
import {
  fetchChatMessages,
  generateChapterContent,
  saveAnnotation,
  sendChatMessage,
} from "@/composables/useStudy";
import ReaderSettings from "@/components/ReaderSettings.vue";
import StudyPanel from "@/components/StudyPanel.vue";
import StudyReader, {
  type StudyStreamState,
} from "@/components/StudyReader.vue";
import type {
  Chapter,
  ChapterAnnotation,
  ChapterDeltaEvent,
  ChatDeltaEvent,
  ChatDoneEvent,
  ChatErrorEvent,
  ChatMessage,
  PlanRecord,
  PlanSummary,
} from "@/types/agent";

/**
 * StudyView：研读页（左章节树 / 中章节内容 / 右标注·笔记·聊天）。
 *
 * 生成口径（后端大纲-only 改版）：空章节是正常态；选中未研读章节即自动
 * 调 generate_chapter_content，正文增量经全局 'chapter-delta' 事件推送，
 * 中栏以打字机渐进揭示（滚出可见区/生成结束转全量 markdown 渲染）；
 * 已生成章节从 get_plan 回读直接渲染。
 *
 * 事件订阅生命周期随本页（App main 分支挂卸）：离开研读页即退订，
 * 后台生成不受影响，重进以 DB 为准。
 *
 * 互斥：生成期间写入 useAgent 的 regenScopes（key 与阶段/章重生成同形），
 * 计划页的重生成按钮经 isRegenInFlight 自动禁用，反之亦然。
 */
const props = defineProps<{
  /**
   * 跨页导航目标（结果页「去研读」入口）：{ planId, seq }，seq 保证同计划
   * 重复跳转也能触发；p/c 可选——携带时进入后直接选中该章节（优先于
   * 上次章节记忆）。
   */
  target: { planId: string; seq: number; p?: number; c?: number } | null;
}>();

// ---------------- 计划与章节选择 ----------------

const plans = ref<PlanSummary[]>([]);
const selectedPlanId = ref<string | null>(null);
/** 已成功加载到 record 的计划 id（v-model 会先改 selectedPlanId，守卫不能只看它） */
const loadedPlanId = ref<string | null>(null);
const record = ref<PlanRecord | null>(null);
const loadingRecord = ref(false);
const recordError = ref<string | null>(null);
/** 当前选中章节（0-based 阶段/章索引；null = 未选） */
const selection = ref<{ p: number; c: number } | null>(null);

function genKey(planId: string, p: number, c: number): string {
  return `${planId}:${p}:${c}`;
}

const currentChapter = computed<Chapter | null>(() => {
  if (!record.value || !selection.value) return null;
  return (
    record.value.plan.phases[selection.value.p]?.chapters?.[
      selection.value.c
    ] ?? null
  );
});

function isSelected(p: number, c: number): boolean {
  return (
    !!selection.value && selection.value.p === p && selection.value.c === c
  );
}

/** 加载计划列表；默认选最近创建的一份（列表返回为收藏置顶序，此处按时间挑） */
async function loadPlans() {
  try {
    plans.value = await fetchPlanSummaries();
    if (!selectedPlanId.value && plans.value.length) {
      const latest = [...plans.value].sort((a, b) =>
        b.createdAt.localeCompare(a.createdAt),
      )[0];
      if (latest) void choosePlan(latest.id);
    }
  } catch (e) {
    recordError.value = normalizeError(e).message;
  }
}

async function choosePlan(
  planId: string,
  focus: { p: number; c: number } | null = null,
) {
  if (loadedPlanId.value === planId && record.value) {
    // 同计划重复「去研读」（早退路径）：仍要落实章节定位
    if (focus && isValidChapter(focus)) selection.value = focus;
    return;
  }
  selectedPlanId.value = planId;
  selection.value = null;
  record.value = null;
  loadedPlanId.value = null;
  recordError.value = null;
  loadingRecord.value = true;
  try {
    record.value = await fetchPlanRecord(planId);
    loadedPlanId.value = planId;
    restoreChapter(planId, focus);
  } catch (e) {
    recordError.value = normalizeError(e).message;
  } finally {
    loadingRecord.value = false;
  }
}

/** 计划下拉切换：直接取事件值，不依赖 v-model 指令的更新时序 */
function onPlanSelect(event: Event) {
  const planId = (event.target as HTMLSelectElement).value;
  if (planId) void choosePlan(planId);
}

function selectChapter(p: number, c: number) {
  selection.value = { p, c };
  if (selectedPlanId.value) saveLastChapter(selectedPlanId.value, { p, c });
  // 窄屏抽屉：选完章收起左树，让位给正文
  leftOpen.value = false;
}

// ---------------- 上次章节记忆（按 planId 独立，localStorage 持久化） ----------------

const LAST_CHAPTER_KEY = "praxis.studyLastChapter";

function isValidChapter(sel: { p: number; c: number }): boolean {
  return !!record.value?.plan.phases[sel.p]?.chapters?.[sel.c];
}

function readLastChapter(planId: string): { p: number; c: number } | null {
  try {
    const map = JSON.parse(
      localStorage.getItem(LAST_CHAPTER_KEY) ?? "{}",
    ) as Record<string, { p: number; c: number }>;
    const saved = map[planId];
    return saved &&
      Number.isInteger(saved.p) &&
      Number.isInteger(saved.c) &&
      saved.p >= 0 &&
      saved.c >= 0
      ? { p: saved.p, c: saved.c }
      : null;
  } catch {
    return null;
  }
}

function saveLastChapter(planId: string, sel: { p: number; c: number }): void {
  try {
    const map = JSON.parse(
      localStorage.getItem(LAST_CHAPTER_KEY) ?? "{}",
    ) as Record<string, { p: number; c: number }>;
    map[planId] = sel;
    localStorage.setItem(LAST_CHAPTER_KEY, JSON.stringify(map));
  } catch {
    // 存储不可用（配额/隐私模式）时静默降级，记忆功能不阻塞研读
  }
}

/** 计划加载完成后恢复定位：跨页 target 的章节优先，其次上次章节记忆 */
function restoreChapter(
  planId: string,
  focus: { p: number; c: number } | null,
): void {
  const restored =
    focus && isValidChapter(focus) ? focus : readLastChapter(planId);
  if (restored && isValidChapter(restored)) selection.value = restored;
}

watch(
  () => props.target,
  (target) => {
    if (!target) return;
    const focus =
      target.p != null && target.c != null
        ? { p: target.p, c: target.c }
        : null;
    void choosePlan(target.planId, focus);
  },
  { immediate: true },
);

onMounted(() => {
  void loadPlans();
  void subscribeEvents();
  document.addEventListener("keydown", onDocKeydown);
});

// ---------------- 按需生成与打字机 ----------------

/** 在飞生成章节集合（key 与 regen scope 同形） */
const pendingKeys = ref<Set<string>>(new Set());

const streamState = ref<StudyStreamState>({
  active: false,
  typewriter: false,
  text: "",
  error: null,
});
let streamBuf = "";
let streamFlushTimer = 0;
/** 中途进入在飞章节：错过的增量无法重放，忽略后续 delta，等终态全量渲染 */
let streamIgnoreDeltas = false;

function onChapterDelta(payload: ChapterDeltaEvent) {
  if (
    !selection.value ||
    payload.planId !== selectedPlanId.value ||
    payload.phaseIndex !== selection.value.p ||
    payload.chapterIndex !== selection.value.c ||
    streamIgnoreDeltas
  ) {
    return;
  }
  streamBuf += payload.text;
  // 节流刷新：增量高频到达时 120ms 一帧，长文不逐 token 重渲染
  if (!streamFlushTimer) {
    streamFlushTimer = window.setTimeout(() => {
      streamFlushTimer = 0;
      streamState.value = { ...streamState.value, text: streamBuf };
    }, 120);
  }
}

async function startGeneration(force = false) {
  const planId = selectedPlanId.value;
  const sel = selection.value;
  const chapter = currentChapter.value;
  if (!planId || !sel || !chapter) return;
  const key = genKey(planId, sel.p, sel.c);
  if (pendingKeys.value.has(key) || (!force && !!chapter.contentMd)) return;
  if (isRegenInFlight(planId)) {
    streamState.value = {
      active: false,
      typewriter: false,
      text: "",
      error: "该计划正在其他位置重新生成，请等它完成后再来研读。",
    };
    return;
  }
  streamBuf = "";
  streamIgnoreDeltas = false;
  streamState.value = { active: true, typewriter: true, text: "", error: null };
  pendingKeys.value.add(key);
  // scope 与阶段/章重生成同形：计划页的生成入口据此互斥禁用
  regenScopes.value.add(key);
  try {
    await generateChapterContent(planId, sel.p, sel.c);
    // 尾段 delta 与终态落库的空窗：稍候再回读权威正文
    await new Promise((resolve) => setTimeout(resolve, 300));
    await refreshRecord();
    streamState.value = {
      active: false,
      typewriter: false,
      text: "",
      error: null,
    };
    streamBuf = "";
  } catch (e) {
    streamState.value = {
      ...streamState.value,
      active: false,
      error: normalizeError(e).message,
    };
  } finally {
    pendingKeys.value.delete(key);
    regenScopes.value.delete(key);
  }
}

async function refreshRecord() {
  const planId = selectedPlanId.value;
  if (!planId) return;
  try {
    record.value = await fetchPlanRecord(planId);
  } catch {
    // 回读失败保留旧记录；终态内容下次进入以 DB 为准
  }
}

/** 章节切换/取消选中：复位流式与聊天视图，空章自动发起生成 */
watch(selection, () => {
  if (streamFlushTimer) {
    clearTimeout(streamFlushTimer);
    streamFlushTimer = 0;
  }
  if (chatFlushTimer) {
    clearTimeout(chatFlushTimer);
    chatFlushTimer = 0;
  }
  chatStreaming.value = null;
  chatError.value = null;
  chatSending.value = false;
  chatBuf = "";
  void loadChatHistory();
  const planId = selectedPlanId.value;
  const sel = selection.value;
  const chapter = currentChapter.value;
  if (!planId || !sel || !chapter) {
    streamIgnoreDeltas = false;
    streamBuf = "";
    streamState.value = {
      active: false,
      typewriter: false,
      text: "",
      error: null,
    };
    return;
  }
  const key = genKey(planId, sel.p, sel.c);
  if (!chapter.contentMd && chapter.status !== "failed") {
    if (pendingKeys.value.has(key)) {
      streamIgnoreDeltas = true;
      streamBuf = "";
      streamState.value = {
        active: true,
        typewriter: false,
        text: "",
        error: null,
      };
      return;
    }
    void startGeneration();
    return;
  }
  streamIgnoreDeltas = false;
  streamBuf = "";
  streamState.value = {
    active: false,
    typewriter: false,
    text: "",
    error: null,
  };
});

/** 左树状态点：ready / generating / failed / unread */
function chapterStatus(
  chapter: Chapter,
  p: number,
  c: number,
): "ready" | "generating" | "failed" | "unread" {
  const planId = selectedPlanId.value;
  if (
    chapter.status === "generating" ||
    (planId && pendingKeys.value.has(genKey(planId, p, c)))
  ) {
    return "generating";
  }
  if (chapter.contentMd) return "ready";
  if (chapter.status === "failed") return "failed";
  return "unread";
}

const DOT_CLASSES: Record<string, string> = {
  ready: "bg-success-4",
  generating: "bg-primary-3 animate-pulse",
  failed: "bg-destructive",
  unread: "bg-muted-foreground/30 ring-1 ring-border",
};

// ---------------- 聊天 ----------------

const chatMessages = ref<ChatMessage[]>([]);
const chatStreaming = ref<string | null>(null);
const chatSending = ref(false);
const chatError = ref<string | null>(null);
let chatBuf = "";
let chatFlushTimer = 0;

function matchesSelection(payload: {
  planId: string;
  phaseIndex: number;
  chapterIndex: number;
}): boolean {
  return (
    !!selection.value &&
    payload.planId === selectedPlanId.value &&
    payload.phaseIndex === selection.value.p &&
    payload.chapterIndex === selection.value.c
  );
}

function onChatDelta(payload: ChatDeltaEvent) {
  if (!matchesSelection(payload)) return;
  chatBuf += payload.text;
  if (chatStreaming.value === null) chatStreaming.value = "";
  // 节流：token 级增量 120ms 一帧渲染，长回答不逐 token 重绘
  if (!chatFlushTimer) {
    chatFlushTimer = window.setTimeout(() => {
      chatFlushTimer = 0;
      if (chatStreaming.value !== null) chatStreaming.value = chatBuf;
    }, 120);
  }
}

function onChatDone(payload: ChatDoneEvent) {
  if (!matchesSelection(payload)) return;
  if (chatFlushTimer) {
    clearTimeout(chatFlushTimer);
    chatFlushTimer = 0;
  }
  chatMessages.value = [...chatMessages.value, payload.message];
  chatStreaming.value = null;
  chatBuf = "";
}

function onChatError(payload: ChatErrorEvent) {
  if (!matchesSelection(payload)) return;
  if (chatFlushTimer) {
    clearTimeout(chatFlushTimer);
    chatFlushTimer = 0;
  }
  chatStreaming.value = null;
  chatBuf = "";
  chatError.value = payload.message;
}

async function loadChatHistory() {
  const planId = selectedPlanId.value;
  const sel = selection.value;
  if (!planId || !sel) {
    chatMessages.value = [];
    return;
  }
  try {
    chatMessages.value = await fetchChatMessages(planId, sel.p, sel.c);
  } catch {
    chatMessages.value = [];
  }
}

async function onSend(question: string) {
  const planId = selectedPlanId.value;
  const sel = selection.value;
  if (!planId || !sel || chatSending.value) return;
  chatMessages.value = [
    ...chatMessages.value,
    { role: "user", content: question },
  ];
  chatSending.value = true;
  chatError.value = null;
  chatBuf = "";
  chatStreaming.value = "";
  try {
    await sendChatMessage(planId, sel.p, sel.c, question);
    // 终态兜底：chat-done 通常先于命令返回到达；万一缺失则对账回读历史
    window.setTimeout(() => {
      if (chatStreaming.value !== null) {
        chatStreaming.value = null;
        chatBuf = "";
        void loadChatHistory();
      }
    }, 500);
  } catch (e) {
    chatStreaming.value = null;
    chatBuf = "";
    chatError.value = normalizeError(e).message;
  } finally {
    chatSending.value = false;
  }
}

// ---------------- 标注（保存入口在中栏，列表在右栏） ----------------

const panelRef = ref<InstanceType<typeof StudyPanel> | null>(null);
const readerRef = ref<InstanceType<typeof StudyReader> | null>(null);

async function onAnnotate(annotation: ChapterAnnotation) {
  const planId = selectedPlanId.value;
  if (!planId) return;
  try {
    await saveAnnotation(planId, annotation);
    await panelRef.value?.refreshAnnotations();
  } catch (e) {
    panelRef.value?.reportAnnotationError(normalizeError(e).message);
  }
}

function onLocate(quote: string) {
  readerRef.value?.locateQuote(quote);
}

// ---------------- 全局事件订阅 ----------------

let unlistens: UnlistenFn[] = [];

async function subscribeEvents() {
  try {
    unlistens = await Promise.all([
      listen<ChapterDeltaEvent>("chapter-delta", (event) =>
        onChapterDelta(event.payload),
      ),
      listen<ChatDeltaEvent>("chat-delta", (event) =>
        onChatDelta(event.payload),
      ),
      listen<ChatDoneEvent>("chat-done", (event) => onChatDone(event.payload)),
      listen<ChatErrorEvent>("chat-error", (event) =>
        onChatError(event.payload),
      ),
    ]);
  } catch {
    // IPC 关停瞬时失败：研读页功能降级为只读浏览，不阻塞页面
  }
}

onUnmounted(() => {
  unlistens.forEach((fn) => fn());
  unlistens = [];
  if (streamFlushTimer) clearTimeout(streamFlushTimer);
  if (chatFlushTimer) clearTimeout(chatFlushTimer);
  document.removeEventListener("keydown", onDocKeydown);
  document.removeEventListener("pointerdown", onSettingsPointerDown, true);
});

// ---------------- 窄屏抽屉 ----------------

const leftOpen = ref(false);
const rightOpen = ref(false);

function closeDrawers() {
  leftOpen.value = false;
  rightOpen.value = false;
}

// ---------------- 阅读偏好与专注模式 ----------------

/** 字号滑块持久化 key（沿用 praxis.* 前缀约定） */
const FONT_KEY = "praxis.studyReaderFont";
/** 行距滑块持久化 key */
const LINE_KEY = "praxis.studyReaderLineHeight";

/** 字号滑块范围（px）：默认 17 与样式表 fallback 一致 */
const FONT_RANGE = { min: 14, max: 22, step: 1, fallback: 17 } as const;
/** 行距滑块范围（无单位倍数）：默认 1.95 与样式表 fallback 一致 */
const LINE_RANGE = { min: 1.6, max: 2.4, step: 0.05, fallback: 1.95 } as const;

/** 滑块范围（min/max/step/fallback）；FONT_RANGE / LINE_RANGE 共用 */
type SliderRange = { min: number; max: number; step: number; fallback: number };

/** 从 localStorage 恢复滑块值：非法回退默认，并对齐步长、钳在范围内（防脏数据） */
function loadRange(key: string, range: SliderRange) {
  const raw = Number(localStorage.getItem(key));
  if (!Number.isFinite(raw)) return range.fallback;
  const aligned =
    range.min + Math.round((raw - range.min) / range.step) * range.step;
  return Math.min(range.max, Math.max(range.min, Number(aligned.toFixed(2))));
}

/** 正文字号（px）与行距（倍数）；经 CSS 变量注入 .reader-body / .markdown-code */
const readerFont = ref(loadRange(FONT_KEY, FONT_RANGE));
const readerLine = ref(loadRange(LINE_KEY, LINE_RANGE));

watch([readerFont, readerLine], ([font, line]) => {
  localStorage.setItem(FONT_KEY, String(font));
  localStorage.setItem(LINE_KEY, String(line));
});

/** 阅读设置弹层开合（页头按钮与专注悬浮条共用） */
const settingsOpen = ref(false);

function onSettingsPointerDown(event: PointerEvent) {
  const target = event.target as HTMLElement | null;
  // 面板与两个触发按钮内部的按压不算「点别处」
  if (
    target?.closest(
      "[data-study-reader-settings], [data-study-settings-btn], [data-study-focus-settings]",
    )
  ) {
    return;
  }
  settingsOpen.value = false;
}

watch(settingsOpen, (open) => {
  if (open) {
    document.addEventListener("pointerdown", onSettingsPointerDown, true);
  } else {
    document.removeEventListener("pointerdown", onSettingsPointerDown, true);
  }
});

/** 专注阅读：复用 App.vue provide 的 readerFocused——.app-shell.is-focused
 * 会隐藏 App 级导航侧栏与顶栏，本页页头/两栏随之收起，正文独占窗口。
 * 切换页面时 App 侧 watch 会自动复位（会话内有效，不持久化）。 */
const readerFocused = inject("readerFocused", ref(false));

/** 退出专注提示：按钮与 Esc 两种退出路径都浮现，短暂停留后自动消失 */
const focusToastVisible = ref(false);
let focusToastTimer: ReturnType<typeof setTimeout> | null = null;

function showFocusToast() {
  focusToastVisible.value = true;
  if (focusToastTimer) clearTimeout(focusToastTimer);
  focusToastTimer = setTimeout(() => {
    focusToastVisible.value = false;
    focusToastTimer = null;
  }, 2200);
}

onUnmounted(() => {
  if (focusToastTimer) clearTimeout(focusToastTimer);
});

async function toggleFocus() {
  readerFocused.value = !readerFocused.value;
  if (readerFocused.value) {
    settingsOpen.value = false;
    closeDrawers();
    return;
  }
  showFocusToast();
  // 与预览页（PlanWorkbench）同约定：退出后焦点还给触发按钮，键盘用户不落回 body
  await nextTick();
  document.querySelector<HTMLButtonElement>('[aria-label="专注阅读"]')?.focus();
}

function onDocKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  // 专注态下 Esc 直接退出（设置弹层随专注一起消失）；否则 Esc 关设置弹层
  if (readerFocused.value) {
    void toggleFocus();
    return;
  }
  if (settingsOpen.value) settingsOpen.value = false;
}
</script>

<template>
  <div
    class="flex h-full min-h-0 flex-col"
    data-study-page
    :style="{
      '--reader-font-size': `${readerFont}px`,
      '--reader-line-height': String(readerLine),
    }"
  >
    <!-- 页头：标题 + 抽屉开关（各自跟随对应栏的常驻断点：左树 lg 起
         常驻、右栏 xl 起常驻；两开关若共用断点，会出“只弹遮罩的无效按钮”） -->
    <header
      v-show="!readerFocused"
      class="flex shrink-0 items-center justify-between gap-3 border-b bg-card px-4 py-2.5 sm:px-6"
    >
      <h1 class="font-display text-xl font-semibold tracking-tight">研读</h1>
      <div class="flex items-center gap-2">
        <!-- 阅读设置：字号 / 行距（弹层面板复用于专注悬浮条） -->
        <div class="relative">
          <button
            type="button"
            data-study-settings-btn
            class="flex min-h-9 items-center gap-1.5 rounded-lg border bg-card px-2.5 text-xs transition-colors hover:border-primary/40 hover:text-primary"
            :aria-expanded="settingsOpen"
            aria-label="阅读设置"
            @click="settingsOpen = !settingsOpen"
          >
            <Type class="h-3.5 w-3.5" aria-hidden="true" />
            Aa
          </button>
          <ReaderSettings
            v-if="settingsOpen"
            class="absolute right-0 top-full z-50 mt-2"
            :font-size="readerFont"
            :font-min="FONT_RANGE.min"
            :font-max="FONT_RANGE.max"
            :line-height="readerLine"
            :line-min="LINE_RANGE.min"
            :line-max="LINE_RANGE.max"
            @update="
              (kind, value) =>
                kind === 'fontSize'
                  ? (readerFont = value)
                  : (readerLine = value)
            "
          />
        </div>
        <!-- 专注阅读：完全沉浸（页头/左右栏隐藏，Esc 或顶部悬浮条退出） -->
        <button
          type="button"
          data-study-focus-btn
          class="flex min-h-9 items-center gap-1.5 rounded-lg border bg-card px-2.5 text-xs transition-colors hover:border-primary/40 hover:text-primary"
          :aria-pressed="readerFocused"
          aria-label="专注阅读"
          @click="toggleFocus"
        >
          <Focus class="h-3.5 w-3.5" aria-hidden="true" />
          专注阅读
        </button>
        <button
          type="button"
          class="flex min-h-9 items-center gap-1.5 rounded-lg border bg-card px-2.5 text-xs transition-colors hover:border-primary/40 hover:text-primary lg:hidden"
          :aria-expanded="leftOpen"
          aria-label="章节目录"
          @click="leftOpen = !leftOpen"
        >
          <BookOpenText class="h-3.5 w-3.5" aria-hidden="true" />
          章节
        </button>
        <button
          type="button"
          class="flex min-h-9 items-center gap-1.5 rounded-lg border bg-card px-2.5 text-xs transition-colors hover:border-primary/40 hover:text-primary xl:hidden"
          :aria-expanded="rightOpen"
          aria-label="伴学面板"
          @click="rightOpen = !rightOpen"
        >
          <MessagesSquare class="h-3.5 w-3.5" aria-hidden="true" />
          伴学
        </button>
      </div>
    </header>

    <!-- 专注模式顶部悬浮条：鼠标移到窗口顶缘浮现（Esc 也可退出）；
         pointer-events 仅 hover 态放开，避免不可见时拦截正文点击 -->
    <div
      v-if="readerFocused"
      data-study-focus-hotspot
      class="group fixed inset-x-0 top-0 z-50 flex h-4 justify-center"
    >
      <div
        data-study-focus-bar
        class="pointer-events-none mt-1 flex h-9 items-center gap-1.5 rounded-full border bg-popover px-2 shadow-xl opacity-0 transition-opacity duration-200 group-hover:pointer-events-auto group-hover:opacity-100"
      >
        <div class="relative">
          <button
            type="button"
            data-study-focus-settings
            class="flex min-h-7 items-center gap-1 rounded-full px-2.5 text-xs transition-colors hover:text-primary"
            :aria-expanded="settingsOpen"
            aria-label="阅读设置"
            @click="settingsOpen = !settingsOpen"
          >
            <Type class="h-3.5 w-3.5" aria-hidden="true" />
            Aa
          </button>
          <ReaderSettings
            v-if="settingsOpen"
            class="absolute left-1/2 top-full z-50 mt-2 -translate-x-1/2"
            :font-size="readerFont"
            :font-min="FONT_RANGE.min"
            :font-max="FONT_RANGE.max"
            :line-height="readerLine"
            :line-min="LINE_RANGE.min"
            :line-max="LINE_RANGE.max"
            @update="
              (kind, value) =>
                kind === 'fontSize'
                  ? (readerFont = value)
                  : (readerLine = value)
            "
          />
        </div>
        <button
          type="button"
          data-study-focus-exit
          class="flex min-h-7 items-center gap-1 rounded-full px-2.5 text-xs transition-colors hover:text-primary"
          aria-label="退出专注"
          @click="toggleFocus"
        >
          <Focus class="h-3.5 w-3.5" aria-hidden="true" />
          退出专注
        </button>
      </div>
    </div>

    <div class="relative flex min-h-0 flex-1 overflow-hidden">
      <!-- 左：计划选择 + 章节树（lg 起常驻；抽屉态宽度由 .study-drawer 钳在主区内） -->
      <aside
        v-show="!readerFocused"
        data-study-tree
        class="study-drawer absolute inset-y-0 left-0 z-40 flex w-64 shrink-0 flex-col border-r bg-card transition-transform duration-200 lg:static lg:z-auto 2xl:w-72"
        :class="
          leftOpen
            ? 'visible translate-x-0 shadow-2xl'
            : 'invisible -translate-x-full lg:visible lg:translate-x-0'
        "
      >
        <div class="shrink-0 border-b p-3">
          <label for="study-plan-select" class="text-xs text-muted-foreground"
            >选择计划</label
          >
          <select
            id="study-plan-select"
            v-model="selectedPlanId"
            data-study-plan-select
            class="mt-1 h-9 w-full rounded-lg border bg-card px-2 text-sm"
            aria-label="选择计划"
            @change="onPlanSelect"
          >
            <option v-for="plan in plans" :key="plan.id" :value="plan.id">
              {{ plan.title }}
            </option>
          </select>
        </div>
        <nav
          class="min-h-0 flex-1 overflow-y-auto px-2 py-3"
          aria-label="章节树"
        >
          <p
            v-if="!plans.length"
            class="px-2 py-6 text-center text-xs leading-relaxed text-muted-foreground"
          >
            还没有学习计划。先到「学习计划」新建一份，再来逐章研读。
          </p>
          <div
            v-for="(phase, p) in record?.plan.phases ?? []"
            :key="p"
            class="mb-3"
          >
            <p
              class="truncate px-2 pb-1 text-xs font-semibold text-muted-foreground"
            >
              {{ p + 1 }}. {{ phase.name }}
            </p>
            <button
              v-for="(chapter, c) in phase.chapters ?? []"
              :key="c"
              type="button"
              v-tip="chapter.title"
              class="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors"
              :class="
                isSelected(p, c)
                  ? 'bg-accent font-medium text-primary'
                  : 'text-foreground/85 hover:bg-muted'
              "
              :data-study-chapter="`${p}:${c}`"
              :aria-label="`研读章节：${chapter.title}`"
              :aria-current="isSelected(p, c) ? 'true' : undefined"
              @click="selectChapter(p, c)"
            >
              <span
                class="h-1.5 w-1.5 shrink-0 rounded-full"
                :class="DOT_CLASSES[chapterStatus(chapter, p, c)]"
                :data-status="chapterStatus(chapter, p, c)"
                aria-hidden="true"
              />
              <span class="min-w-0 flex-1 truncate">{{ chapter.title }}</span>
              <span
                v-if="chapter.week != null"
                class="shrink-0 rounded bg-secondary-2 px-1.5 py-0.5 text-[10px] tabular-nums text-secondary-foreground"
                >第 {{ chapter.week }} 周</span
              >
            </button>
          </div>
        </nav>
      </aside>

      <!-- 中：章节内容（含打字机与标注交互）；lg 起 480px 下限防三栏挤压坍缩 -->
      <div
        class="relative flex min-h-0 min-w-0 flex-1 flex-col lg:min-w-[480px]"
      >
        <StudyReader
          ref="readerRef"
          :chapter="currentChapter"
          :phase-index="selection?.p ?? null"
          :chapter-index="selection?.c ?? null"
          :stream="streamState"
          @retry="startGeneration(true)"
          @annotate="onAnnotate"
        />
        <!-- 抽屉遮罩：只盖中栏正文（fixed 全屏会连侧栏/页头一起盖住），
             左右抽屉 z-40 在其上不受影响；点击正文任意处收起抽屉 -->
        <div
          v-if="leftOpen || rightOpen"
          class="absolute inset-0 z-20 bg-foreground/25 xl:hidden"
          aria-hidden="true"
          @click="closeDrawers"
        />
        <!-- 计划加载/失败覆盖层 -->
        <div
          v-if="recordError"
          class="absolute inset-0 z-10 flex items-center justify-center bg-background p-6"
        >
          <div
            class="max-w-sm rounded-lg border border-destructive/40 bg-destructive-1 px-4 py-5 text-center"
            role="alert"
          >
            <TriangleAlert
              class="mx-auto h-6 w-6 text-destructive"
              aria-hidden="true"
            />
            <p class="mt-2 break-all text-sm text-destructive">
              {{ recordError }}
            </p>
          </div>
        </div>
        <div
          v-else-if="loadingRecord && !record"
          class="absolute inset-0 z-10 flex items-center justify-center bg-background"
        >
          <div
            class="h-9 w-9 animate-spin rounded-full border-2 border-primary border-t-transparent"
            role="status"
            aria-label="正在加载计划"
          />
        </div>
      </div>

      <!-- 右：标注 / 笔记 / 聊天（xl 起常驻；1280 以下走抽屉，见 P0-1 断点评审） -->
      <aside
        v-show="!readerFocused"
        class="study-drawer absolute inset-y-0 right-0 z-40 flex w-80 shrink-0 flex-col border-l bg-card transition-transform duration-200 xl:static xl:z-auto 2xl:w-[360px]"
        :class="
          rightOpen
            ? 'visible translate-x-0 shadow-2xl'
            : 'invisible translate-x-full xl:visible xl:translate-x-0'
        "
      >
        <StudyPanel
          ref="panelRef"
          class="min-h-0 w-full flex-1"
          :plan-id="selectedPlanId ?? ''"
          :phase-index="selection?.p ?? 0"
          :chapter-index="selection?.c ?? 0"
          :chapter-title="currentChapter?.title ?? '未选择章节'"
          :chat-messages="chatMessages"
          :chat-streaming="chatStreaming"
          :chat-error="chatError"
          :chat-sending="chatSending"
          @send="onSend"
          @locate="onLocate"
        />
      </aside>
    </div>

    <!-- 退出专注提示：按钮与 Esc 两种退出路径共用，浮现后自动消失 -->
    <div
      data-study-focus-toast
      class="pointer-events-none fixed left-1/2 top-6 z-[60] -translate-x-1/2 transition-all duration-300"
      :class="
        focusToastVisible
          ? 'translate-y-0 opacity-100'
          : '-translate-y-2 opacity-0'
      "
      role="status"
      aria-live="polite"
    >
      <div
        class="flex items-center gap-2 rounded-full border bg-popover/95 px-4 py-2 text-sm shadow-xl backdrop-blur"
      >
        <CheckCircle2 class="h-4 w-4 text-primary" aria-hidden="true" />
        已退出专注模式
      </div>
    </div>
  </div>
</template>
