<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { BookOpenText, MessagesSquare, TriangleAlert } from "lucide-vue-next";
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
  /** 跨页导航目标（结果页「去研读」入口）：{ planId, seq }，seq 保证同计划重复跳转也能触发 */
  target: { planId: string; seq: number } | null;
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
  return !!selection.value && selection.value.p === p && selection.value.c === c;
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

async function choosePlan(planId: string) {
  if (loadedPlanId.value === planId && record.value) return;
  selectedPlanId.value = planId;
  selection.value = null;
  record.value = null;
  loadedPlanId.value = null;
  recordError.value = null;
  loadingRecord.value = true;
  try {
    record.value = await fetchPlanRecord(planId);
    loadedPlanId.value = planId;
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
  // 窄屏抽屉：选完章收起左树，让位给正文
  leftOpen.value = false;
}

watch(
  () => props.target,
  (target) => {
    if (target) void choosePlan(target.planId);
  },
  { immediate: true },
);

onMounted(() => {
  void loadPlans();
  void subscribeEvents();
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
  streamState.value = { active: false, typewriter: false, text: "", error: null };
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
});

// ---------------- 窄屏抽屉 ----------------

const leftOpen = ref(false);
const rightOpen = ref(false);

function closeDrawers() {
  leftOpen.value = false;
  rightOpen.value = false;
}
</script>

<template>
  <div class="flex h-full min-h-0 flex-col" data-study-page>
    <!-- 页头：标题 + 窄屏抽屉开关（三栏在 lg 及以上整列展开） -->
    <header
      class="flex shrink-0 items-center justify-between gap-3 border-b bg-card px-4 py-2.5 sm:px-6"
    >
      <h1 class="font-display text-xl font-semibold tracking-tight">研读</h1>
      <div class="flex items-center gap-2 lg:hidden">
        <button
          type="button"
          class="flex min-h-9 items-center gap-1.5 rounded-lg border bg-card px-2.5 text-xs transition-colors hover:border-primary/40 hover:text-primary"
          :aria-expanded="leftOpen"
          aria-label="章节目录"
          @click="leftOpen = !leftOpen"
        >
          <BookOpenText class="h-3.5 w-3.5" aria-hidden="true" />
          章节
        </button>
        <button
          type="button"
          class="flex min-h-9 items-center gap-1.5 rounded-lg border bg-card px-2.5 text-xs transition-colors hover:border-primary/40 hover:text-primary"
          :aria-expanded="rightOpen"
          aria-label="伴学面板"
          @click="rightOpen = !rightOpen"
        >
          <MessagesSquare class="h-3.5 w-3.5" aria-hidden="true" />
          伴学
        </button>
      </div>
    </header>

    <div class="relative flex min-h-0 flex-1 overflow-hidden">
      <!-- 窄屏抽屉遮罩 -->
      <div
        v-if="leftOpen || rightOpen"
        class="fixed inset-0 z-30 bg-foreground/25 lg:hidden"
        aria-hidden="true"
        @click="closeDrawers"
      />

      <!-- 左：计划选择 + 章节树（窄屏抽屉化，可折叠） -->
      <aside
        data-study-tree
        class="absolute inset-y-0 left-0 z-40 flex w-72 max-w-[85vw] shrink-0 flex-col border-r bg-card transition-transform duration-200 lg:static lg:z-auto"
        :class="
          leftOpen
            ? 'visible translate-x-0 shadow-2xl'
            : 'invisible -translate-x-full lg:visible lg:translate-x-0'
        "
      >
        <div class="shrink-0 border-b p-3">
          <label
            for="study-plan-select"
            class="text-xs text-muted-foreground"
          >选择计划</label>
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
                aria-hidden="true"
              />
              <span class="min-w-0 flex-1 truncate">{{ chapter.title }}</span>
              <span
                v-if="chapter.week != null"
                class="shrink-0 rounded bg-secondary-2 px-1.5 py-0.5 text-[10px] tabular-nums text-secondary-foreground"
              >第 {{ chapter.week }} 周</span>
            </button>
          </div>
        </nav>
      </aside>

      <!-- 中：章节内容（含打字机与标注交互） -->
      <div class="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <StudyReader
          ref="readerRef"
          :chapter="currentChapter"
          :phase-index="selection?.p ?? null"
          :chapter-index="selection?.c ?? null"
          :stream="streamState"
          @retry="startGeneration(true)"
          @annotate="onAnnotate"
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

      <!-- 右：标注 / 笔记 / 聊天（窄屏抽屉化） -->
      <aside
        class="absolute inset-y-0 right-0 z-40 flex w-[360px] max-w-[88vw] shrink-0 flex-col border-l bg-card transition-transform duration-200 lg:static lg:z-auto"
        :class="
          rightOpen
            ? 'visible translate-x-0 shadow-2xl'
            : 'invisible translate-x-full lg:visible lg:translate-x-0'
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
  </div>
</template>
