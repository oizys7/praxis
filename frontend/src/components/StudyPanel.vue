<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import {
  Highlighter,
  NotebookPen,
  Send,
  MessagesSquare,
  Trash2,
} from "lucide-vue-next";
import {
  deleteAnnotation,
  fetchAnnotations,
  fetchChapterNote,
  saveChapterNote,
} from "@/composables/useStudy";
import { normalizeError } from "@/composables/useAgent";
import type { ChapterAnnotation, ChatMessage } from "@/types/agent";

/**
 * StudyPanel：研读页右栏——标注 / 笔记 / 聊天三个分区（选项卡组织）。
 * 刻意安静：默认落在聊天分区，列表类内容一律轻量样式，不抢中栏阅读焦点。
 *
 * 数据归属：
 * - 标注 / 笔记：本组件自管（加载、删除、防抖保存）；保存入口在中栏
 *   （选区标注），由父组件调 refreshAnnotations() 回写列表；
 * - 聊天：消息与流式回复由父组件统一经全局事件维护（跨章节独立），
 *   本组件只渲染 + 发送输入。
 */
const props = defineProps<{
  planId: string;
  phaseIndex: number;
  chapterIndex: number;
  chapterTitle: string;
  /** 聊天消息流（父组件维护：历史 + 已确认回答） */
  chatMessages: ChatMessage[];
  /** 流式中的回答文本（null = 无在途回答） */
  chatStreaming: string | null;
  /** 聊天失败提示（chat-error 终态） */
  chatError: string | null;
  /** 发送中（提问已提交、回答未终态）：输入禁用防连发 */
  chatSending: boolean;
}>();

const emit = defineEmits<{
  (e: "send", question: string): void;
  (e: "locate", quote: string): void;
}>();

/** 分区：quiet 选项卡，默认聊天 */
const activeTab = ref<"chat" | "annotations" | "note">("chat");
const TABS = [
  { id: "chat", label: "聊天", icon: MessagesSquare },
  { id: "annotations", label: "标注", icon: Highlighter },
  { id: "note", label: "笔记", icon: NotebookPen },
] as const;

/**
 * 选项卡键盘导航（WAI-ARIA tabs 模式）：左右方向键循环切换，Home/End
 * 直达首尾；roving tabindex——激活页签可 Tab 进入，其余 -1 跳过。
 */
function onTabKeydown(event: KeyboardEvent) {
  const ids = TABS.map((tab) => tab.id);
  const current = ids.indexOf(activeTab.value);
  let next = -1;
  if (event.key === "ArrowRight") next = (current + 1) % ids.length;
  else if (event.key === "ArrowLeft")
    next = (current - 1 + ids.length) % ids.length;
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = ids.length - 1;
  if (next < 0) return;
  event.preventDefault();
  const id = ids[next];
  if (!id) return;
  activeTab.value = id;
  void nextTick(() => document.getElementById(`study-tab-${id}`)?.focus());
}

// ---------------- 标注 ----------------

const annotations = ref<ChapterAnnotation[]>([]);
const annotationsLoading = ref(false);
const annotationError = ref<string | null>(null);

async function refreshAnnotations() {
  annotationsLoading.value = true;
  annotationError.value = null;
  try {
    annotations.value = await fetchAnnotations(
      props.planId,
      props.phaseIndex,
      props.chapterIndex,
    );
  } catch (e) {
    annotationError.value = normalizeError(e).message;
  } finally {
    annotationsLoading.value = false;
  }
}

async function onRemoveAnnotation(id: string) {
  try {
    await deleteAnnotation(
      props.planId,
      props.phaseIndex,
      props.chapterIndex,
      id,
    );
    annotations.value = annotations.value.filter((item) => item.id !== id);
  } catch (e) {
    annotationError.value = normalizeError(e).message;
  }
}

function formatTime(iso?: string): string {
  if (!iso) return "";
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleString("zh-CN", {
        month: "numeric",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
}

// ---------------- 笔记（章节级 markdown，防抖保存） ----------------

const noteText = ref("");
const noteStatus = ref<"idle" | "dirty" | "saving" | "saved">("idle");
const noteSavedAt = ref("");
const noteError = ref<string | null>(null);
let noteTimer: ReturnType<typeof setTimeout> | undefined;
/** 章节切换的加载互斥 key：迟到的读取不覆盖新章节的笔记 */
let noteLoadedKey = "";
/** 输入时快照的草稿与目标章节：防抖期间切换章节不丢字、不写错章 */
let pendingDraft = "";
let pendingTarget: {
  planId: string;
  phaseIndex: number;
  chapterIndex: number;
} | null = null;

/** 章节切换的加载互斥 key */
function chapterKey(): string {
  return `${props.planId}:${props.phaseIndex}:${props.chapterIndex}`;
}

async function loadNote() {
  const key = chapterKey();
  const { planId, phaseIndex, chapterIndex } = props;
  noteLoadedKey = key;
  noteStatus.value = "idle";
  noteError.value = null;
  if (!planId) {
    noteText.value = "";
    return;
  }
  try {
    const note = await fetchChapterNote(planId, phaseIndex, chapterIndex);
    if (noteLoadedKey !== key) return;
    noteText.value = note?.contentMd ?? "";
  } catch (e) {
    if (noteLoadedKey !== key) return;
    noteError.value = normalizeError(e).message;
  }
}

watch(
  () => [props.planId, props.phaseIndex, props.chapterIndex] as const,
  () => {
    // 未落盘的草稿随切换立即冲掉（用输入时快照，不读已重置的 textarea）
    if (noteTimer) {
      clearTimeout(noteTimer);
      noteTimer = undefined;
      void persistNote();
    }
    void loadNote();
  },
  { immediate: true },
);

function onNoteInput() {
  noteStatus.value = "dirty";
  pendingDraft = noteText.value;
  pendingTarget = {
    planId: props.planId,
    phaseIndex: props.phaseIndex,
    chapterIndex: props.chapterIndex,
  };
  if (noteTimer) clearTimeout(noteTimer);
  noteTimer = setTimeout(() => void persistNote(), 700);
}

async function persistNote() {
  noteTimer = undefined;
  const target = pendingTarget;
  const content = pendingDraft;
  pendingTarget = null;
  if (!target) return;
  noteStatus.value = "saving";
  try {
    await saveChapterNote(
      target.planId,
      target.phaseIndex,
      target.chapterIndex,
      content,
    );
    if (pendingTarget || noteLoadedKey !== chapterKey()) return;
    noteStatus.value = "saved";
    noteSavedAt.value = new Date().toLocaleTimeString("zh-CN", {
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch (e) {
    if (pendingTarget || noteLoadedKey !== chapterKey()) return;
    noteStatus.value = "dirty";
    noteError.value = normalizeError(e).message;
  }
}

onBeforeUnmount(() => {
  // 卸载前把未落盘的草稿冲掉（防抖尾巴不丢字）
  if (noteTimer) {
    clearTimeout(noteTimer);
    void persistNote();
  }
});

// ---------------- 聊天 ----------------

const question = ref("");
const chatListEl = ref<HTMLElement | null>(null);
/** 用户是否滚离底部：滚离时不强制跟随，回到底部恢复 */
let chatFollowing = true;

function onChatScroll() {
  const el = chatListEl.value;
  if (!el) return;
  chatFollowing = el.scrollTop + el.clientHeight >= el.scrollHeight - 120;
}

async function scrollChatToBottom() {
  await nextTick();
  if (!chatFollowing || !chatListEl.value) return;
  chatListEl.value.scrollTop = chatListEl.value.scrollHeight;
}

watch(
  () => [props.chatMessages.length, props.chatStreaming] as const,
  () => void scrollChatToBottom(),
);

function submitQuestion() {
  const text = question.value.trim();
  if (!text || props.chatSending) return;
  question.value = "";
  chatFollowing = true;
  emit("send", text);
}

defineExpose({
  refreshAnnotations,
  activeTab,
  /** 中栏保存标注失败时由父组件回灌错误（错误展示位在本组件） */
  reportAnnotationError(message: string) {
    annotationError.value = message;
  },
});
</script>

<template>
  <div class="flex min-h-0 flex-col" data-study-panel>
    <!-- 分区选项卡：quiet，正文权重永远在中栏（tabs 键盘模型见 onTabKeydown） -->
    <div
      class="flex shrink-0 items-center gap-1 border-b px-3 py-2"
      role="tablist"
      aria-label="研读工具"
      @keydown="onTabKeydown"
    >
      <button
        v-for="tab in TABS"
        :id="`study-tab-${tab.id}`"
        :key="tab.id"
        type="button"
        role="tab"
        class="flex min-h-9 items-center gap-1.5 rounded-lg px-2.5 text-xs transition-colors"
        :class="
          activeTab === tab.id
            ? 'bg-accent font-semibold text-primary'
            : 'text-muted-foreground hover:bg-muted hover:text-foreground'
        "
        :aria-selected="activeTab === tab.id"
        :aria-controls="`study-tabpanel-${tab.id}`"
        :tabindex="activeTab === tab.id ? 0 : -1"
        :data-study-tab="tab.id"
        @click="activeTab = tab.id"
      >
        <component :is="tab.icon" class="h-3.5 w-3.5" aria-hidden="true" />
        {{ tab.label }}
        <span
          v-if="tab.id === 'annotations' && annotations.length"
          class="rounded-full bg-secondary-4 px-1.5 text-[10px] leading-4 text-secondary-foreground"
          >{{ annotations.length }}</span
        >
      </button>
    </div>
    <p
      class="shrink-0 truncate border-b bg-muted-1 px-3 py-1.5 text-xs text-muted-foreground"
    >
      {{ chapterTitle }}
    </p>

    <!-- 聊天 -->
    <div
      v-show="activeTab === 'chat'"
      id="study-tabpanel-chat"
      role="tabpanel"
      aria-labelledby="study-tab-chat"
      class="flex min-h-0 flex-1 flex-col"
    >
      <div
        ref="chatListEl"
        class="min-h-0 flex-1 space-y-3 overflow-y-auto px-3 py-3"
        data-study-chat-list
        @scroll="onChatScroll"
      >
        <p
          v-if="!chatMessages.length && !chatStreaming"
          class="px-1 py-6 text-center text-xs leading-relaxed text-muted-foreground"
        >
          针对本章内容提问，Agent 会带着章节上下文回答。历史对话按章节独立保存。
        </p>
        <!-- 用户消息用 3 阶基准色底（亮色下 2 阶近纸面、气泡不显形），
             明暗两主题「3 = 族基准」语义一致 -->
        <div
          v-for="(message, index) in chatMessages"
          :key="message.id ?? `m${index}`"
          data-study-chat-message
          class="flex"
          :class="message.role === 'user' ? 'justify-end' : 'justify-start'"
        >
          <div
            class="max-w-[85%] whitespace-pre-wrap break-words rounded-xl px-3 py-2 text-sm leading-relaxed"
            :class="
              message.role === 'user'
                ? 'bg-primary-3 text-primary-foreground'
                : 'bg-muted text-foreground'
            "
          >
            {{ message.content }}
          </div>
        </div>
        <!-- 流式回答：纯文本增量（rAF 节流由父侧缓冲承担） -->
        <div v-if="chatStreaming !== null" class="flex justify-start">
          <div
            data-study-chat-streaming
            class="max-w-[85%] whitespace-pre-wrap break-words rounded-xl bg-muted px-3 py-2 text-sm leading-relaxed text-foreground"
          >
            {{ chatStreaming
            }}<span
              class="ml-0.5 inline-block h-3 w-[2px] animate-pulse bg-primary align-middle"
              aria-hidden="true"
            />
          </div>
        </div>
        <p
          v-if="chatError"
          class="break-all rounded-lg bg-destructive-1 px-2.5 py-2 text-xs text-destructive"
          role="alert"
        >
          {{ chatError }}
        </p>
      </div>
      <form class="shrink-0 border-t p-3" @submit.prevent="submitQuestion">
        <div class="flex items-end gap-2">
          <textarea
            v-model="question"
            rows="2"
            data-study-chat-input
            aria-label="向 Agent 提问"
            placeholder="针对本章提问…"
            class="max-h-32 min-h-0 flex-1 resize-none rounded-lg border bg-card px-2.5 py-2 text-sm outline-none focus:ring-2 focus:ring-ring"
            :disabled="chatSending"
            @keydown.enter.exact.prevent="submitQuestion"
          />
          <button
            type="submit"
            aria-label="发送"
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground transition-colors hover:bg-primary-4 disabled:opacity-50"
            :disabled="!question.trim() || chatSending"
          >
            <Send class="h-4 w-4" aria-hidden="true" />
          </button>
        </div>
      </form>
    </div>

    <!-- 标注 -->
    <div
      v-show="activeTab === 'annotations'"
      id="study-tabpanel-annotations"
      role="tabpanel"
      aria-labelledby="study-tab-annotations"
      class="min-h-0 flex-1 overflow-y-auto px-3 py-3"
    >
      <p
        v-if="annotationError"
        class="mb-2 break-all rounded-lg bg-destructive-1 px-2.5 py-2 text-xs text-destructive"
      >
        {{ annotationError }}
      </p>
      <p
        v-if="!annotations.length"
        class="px-1 py-6 text-center text-xs leading-relaxed text-muted-foreground"
      >
        在中栏选中一段文字即可添加标注；点击条目可跳回原文。
      </p>
      <ul class="space-y-2" data-study-annotation-list>
        <li
          v-for="item in annotations"
          :key="item.id"
          data-study-annotation-item
          class="group rounded-lg border bg-card px-2.5 py-2"
        >
          <button
            type="button"
            class="block w-full text-left"
            :aria-label="`定位标注：${item.quote.slice(0, 20)}`"
            @click="emit('locate', item.quote)"
          >
            <span
              class="line-clamp-3 border-l-2 border-primary-3 pl-2 text-xs leading-relaxed text-foreground/85"
              >{{ item.quote }}</span
            >
            <span
              v-if="item.comment"
              class="mt-1 block line-clamp-2 pl-2 text-xs leading-relaxed text-muted-foreground"
              >{{ item.comment }}</span
            >
          </button>
          <div
            class="mt-1 flex items-center justify-between pl-2 text-[10px] text-muted-foreground"
          >
            <span>{{ formatTime(item.createdAt) }}</span>
            <button
              type="button"
              class="rounded p-1 opacity-0 transition-opacity hover:bg-destructive-1 hover:text-destructive focus-visible:opacity-100 group-hover:opacity-100"
              :aria-label="`删除标注：${item.quote.slice(0, 20)}`"
              @click="onRemoveAnnotation(item.id)"
            >
              <Trash2 class="h-3 w-3" aria-hidden="true" />
            </button>
          </div>
        </li>
      </ul>
    </div>

    <!-- 笔记：章节级 markdown，防抖 700ms 自动保存 -->
    <div
      v-show="activeTab === 'note'"
      id="study-tabpanel-note"
      role="tabpanel"
      aria-labelledby="study-tab-note"
      class="flex min-h-0 flex-1 flex-col px-3 py-3"
    >
      <textarea
        v-model="noteText"
        data-study-note-input
        aria-label="章节笔记（markdown）"
        placeholder="记录这一章的思考，markdown 语法，自动保存。"
        class="min-h-0 w-full flex-1 resize-none rounded-lg border bg-card px-2.5 py-2 text-sm leading-relaxed outline-none focus:ring-2 focus:ring-ring"
        @input="onNoteInput"
      />
      <p
        class="mt-2 flex h-4 shrink-0 items-center gap-1.5 text-[10px] text-muted-foreground"
        role="status"
        data-study-note-status
      >
        <template v-if="noteError">
          <span class="text-destructive">{{ noteError }}</span>
        </template>
        <template v-else-if="noteStatus === 'saving'">保存中…</template>
        <template v-else-if="noteStatus === 'saved'">
          已保存{{ noteSavedAt ? ` ${noteSavedAt}` : "" }}
        </template>
        <template v-else-if="noteStatus === 'dirty'">编辑中…</template>
      </p>
    </div>
  </div>
</template>
