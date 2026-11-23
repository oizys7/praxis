<script setup lang="ts">
import { computed, inject, ref, watch, type ShallowRef } from "vue";
import { BookOpenText } from "lucide-vue-next";
import { marked } from "marked";
import MarkdownContent from "@/components/MarkdownContent.vue";
import VisualizationPlayer from "@/components/VisualizationPlayer.vue";
import ExercisePanel from "@/components/ExercisePanel.vue";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  isRegenInFlight,
  isScopeInFlight,
  normalizeError,
  phaseScope,
  regenerateChapter,
} from "@/composables/useAgent";
import { fetchPlanRecord } from "@/composables/usePlanHistory";
import { isChapterGenerating } from "@/composables/useGenerationProgress";
import { readerStickyOffset, scrollToWithinMain } from "@/lib/utils";
import type { Chapter, StudyPlan } from "@/types/agent";

/**
 * ChapterSection：渲染单个学习章节（Chapter）。
 * 整章标题行折叠（默认展开，chevron 切换正文 v-show）→
 * 正文走 MarkdownContent（marked + DOMPurify，mermaid 码块内联渲染）。
 * 头部按 chapter.status 渲染章节级徽标（undefined = 旧数据无徽标）；
 * failed/pending 且 planId 可用时提供「重新生成此章」入口。
 */
const props = defineProps<{
  chapter: Chapter;
  /** 历史计划 id：重生成入口依赖，缺省（新生成结果）不显示按钮 */
  planId?: string;
  /** 所属阶段索引（0-based，重生成命令参数） */
  phaseIndex?: number;
  /** 章节索引（0-based，重生成命令参数） */
  chapterIndex?: number;
  /** 行级生成状态（generating/failed/complete）：驱动「生成中」徽章与重生成抑制 */
  rowStatus?: string;
  /** 所属阶段状态（generating = 阶段重生成中，反馈④逐章动画上下文） */
  phaseStatus?: string;
  /** 全局重生成互斥（反馈③）：计划内任一阶段/章节 generating 时全站入口禁用 */
  regenLocked?: boolean;
}>();

/** 重生成完成：刷新后的计划整体上抛供调用方替换 */
const emit = defineEmits<{ refreshed: [plan: StudyPlan] }>();

/** 所属阶段级重生成是否在飞（阶段按钮发起，本阶段章节由后端统一重做） */
const phaseRegenInFlight = computed(
  () =>
    !!props.planId &&
    props.phaseIndex !== undefined &&
    isScopeInFlight(phaseScope(props.planId, props.phaseIndex)),
);

/** 同计划任意重生成是否在飞（跨 scope 互斥：生成结束前所有生成入口不可点） */
const planRegenInFlight = computed(
  () => !!props.planId && isRegenInFlight(props.planId),
);

/** 章型标签：让读者一眼看出这章是读原理还是动手做（旧数据无 kind 不显示） */
const KIND_LABELS: Record<string, string> = {
  concept: "概念",
  practice: "实操",
  project: "项目",
  review: "复习",
};
const kindLabel = computed(() => KIND_LABELS[props.chapter.kind ?? ""] ?? null);

/** 章节级徽标仅 failed/pending：complete 章节照常渲染不额外加标 */
const statusBadge = computed<{
  label: string;
  variant: "secondary" | "destructive";
} | null>(() => {
  switch (props.chapter.status) {
    case "generating":
      // 重生成落库的持久态（反馈①）或 chapterStart 事件点亮（反馈④）
      return { label: "⟳ 生成中", variant: "secondary" };
    case "failed":
      // 阶段重生成中失败章节会被重做 → 显示生成中而非失败
      return phaseRegenInFlight.value || chapterRegenerating.value
        ? { label: "⟳ 生成中", variant: "secondary" }
        : { label: "✗ 失败", variant: "destructive" };
    case "pending":
      // 行还在 generating 时 pending ≠ 未开始，而是「正在生成本章节」（反馈③区分）
      return props.rowStatus === "generating" ||
        phaseRegenInFlight.value ||
        chapterRegenerating.value
        ? { label: "⟳ 生成中", variant: "secondary" }
        : unreadNewStyle.value
          ? { label: "未研读", variant: "secondary" }
          : { label: "◌ 未生成", variant: "secondary" };
    default:
      return null;
  }
});

/** 本章节重生成按钮：仅历史详情（有 planId）且目标处于 failed/pending 态时
 * 可用；行仍在 generating 时不开放（与后台生成竞争）。大纲-only 新口径的
 * 未研读章不在此列——按需生成的主入口在研读页 */
const canRegenerate = computed(
  () =>
    !!props.planId &&
    props.rowStatus !== "generating" &&
    props.chapter.status !== "generating" &&
    (props.chapter.status === "failed" || props.chapter.status === "pending") &&
    !unreadNewStyle.value,
);

/** 目录命令信箱（悬浮目录 → provide）：点章节时确保本章节正文展开（反馈①） */
const tocCommands = inject<ShallowRef<{
  phase?: number;
  chapter?: { p: number; c: number };
  seq: number;
} | null> | null>("planTocCommands", null);
watch(
  () => tocCommands?.value,
  (cmd) => {
    if (
      cmd?.chapter &&
      cmd.chapter.p === props.phaseIndex &&
      cmd.chapter.c === props.chapterIndex
    ) {
      collapsed.value = false;
    }
  },
);

/**
 * 跨页「去研读」入口（App provide）：大纲-only 改版后空正文是正常态，
 * 主入口从「重新生成此章」让位给研读页的按需生成。
 */
const openStudy = inject<((planId: string) => void) | null>("openStudy", null);

/**
 * 大纲-only 新口径：空正文 + 大纲已分配周次（week != null）且非生成中/失败
 * =「未研读」正常态。旧计划（week == null 的空章）保持「未生成 + 重新生成」
 * 行为不变。
 */
const unreadNewStyle = computed(
  () =>
    !props.chapter.contentMd &&
    props.chapter.week != null &&
    props.chapter.status !== "generating" &&
    props.chapter.status !== "failed",
);

/** 去研读：跳研读页并预选本计划 */
function goStudy() {
  if (props.planId) openStudy?.(props.planId);
}

/**
 * 本章节 scope 是否在飞（keyed 锁判据，task-D2 迁移：key 形态与
 * regenerateChapter 模块级 scope "{planId}:{phaseIndex}:{chapterIndex}"
 * 逐字一致）。同一会话页面上一定会锁住同章节的目标位，同计划其它章
 * 互不影响（原全局布尔 regenInFlight 的偏严互斥已退役）。
 */
const regenScopeInFlight = computed(() =>
  isScopeInFlight(`${props.planId}:${props.phaseIndex}:${props.chapterIndex}`),
);

/** 本组件自身点击发起的在飞标志（供 finally 与守卫双保险） */
const busy = ref(false);
/** 本章是否正在生成中（2026-10-16 反馈④）：chapterStart 事件点亮（内存）
 * ∪ DB status=generating（持久，退出重进保持）∪ 本组件点击乐观 */
const chapterRegenerating = computed(
  () =>
    isChapterGenerating(props.planId, props.phaseIndex, props.chapterIndex) ||
    props.chapter.status === "generating" ||
    busy.value ||
    regenScopeInFlight.value,
);
/** 重生成过程的反馈信息（入库提示 / 失败原因；null = 无） */
const regenError = ref<string | null>(null);

/** 重生成此章：成败都刷新详情（失败也可见最新 failed 态），刷新后整体上抛 */
async function onRegenerate() {
  if (
    !props.planId ||
    busy.value ||
    regenScopeInFlight.value ||
    props.phaseIndex === undefined ||
    props.chapterIndex === undefined
  ) {
    return;
  }
  busy.value = true;
  regenError.value = null;
  try {
    const envelope = await regenerateChapter(
      props.planId,
      props.phaseIndex,
      props.chapterIndex,
      (event) => {
        // 入库/收尾异常即时展示，不打断流程（PhaseDone{total=1} 不展示 1/1 计数）
        if (event.type === "storageIssue") regenError.value = event.message;
      },
    );
    const record = await fetchPlanRecord(props.planId);
    emit("refreshed", record.plan);
    if (envelope.status !== "plan" && envelope.error) {
      regenError.value = envelope.error.message;
    }
  } catch (e) {
    const err = normalizeError(e);
    try {
      // 传输错误同样刷新：Rust 收尾已把目标行翻 failed，详情可见最新状态
      emit("refreshed", (await fetchPlanRecord(props.planId)).plan);
    } catch {
      // 详情读取失败时仅展示错误，不打断原有渲染
    }
    regenError.value = err.message;
  } finally {
    busy.value = false;
  }
}

/** 整章折叠态：默认展开（重生成入口在标题行，折叠不影响操作） */
const collapsed = ref(false);

/** 章内小节目录：从 contentMd 的 ## / ### 行提取（不依赖渲染后的 DOM） */
const headings = computed(() => {
  const list: { level: number; text: string }[] = [];
  if (!props.chapter.contentMd) return list;
  marked.walkTokens(marked.lexer(props.chapter.contentMd), (token) => {
    if (token.type === "heading" && (token.depth === 2 || token.depth === 3)) {
      const html = marked.parseInline(token.text, { async: false }) as string;
      const text =
        new DOMParser().parseFromString(html, "text/html").body.textContent ??
        token.text;
      list.push({ level: token.depth, text });
    }
  });
  return list;
});

const contentRoot = ref<HTMLElement | null>(null);

/**
 * 按索引滚动到对应小节标题。
 * 正文由 marked 渲染，标题渲染顺序与上方提取顺序一致，故索引可直接对齐；
 * 这样无需生成/维护锚点 id（省掉一套 slug 冲突处理）。
 */
function scrollToHeading(index: number) {
  const nodes = contentRoot.value?.querySelectorAll<HTMLElement>("h2, h3");
  scrollToWithinMain(nodes?.[index] ?? null, readerStickyOffset(12));
}
</script>

<template>
  <section class="reader-chapter">
    <!-- 整章标题行：chevron 切换正文显示；右侧章节级徽标与重生成入口 -->
    <header class="flex items-start justify-between gap-2">
      <button
        type="button"
        class="flex min-h-11 min-w-0 flex-1 items-start gap-3 rounded text-left"
        :aria-expanded="!collapsed"
        @click="collapsed = !collapsed"
      >
        <svg
          class="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200"
          :class="collapsed ? '' : 'rotate-180'"
          aria-hidden="true"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="m6 9 6 6 6-6" />
        </svg>
        <div class="min-w-0">
          <h4
            class="flex flex-wrap items-baseline gap-2 text-xl font-semibold leading-relaxed"
          >
            <span>{{ props.chapter.title }}</span>
            <!-- 打卡态是只读派生（由打卡记录聚合），这里只负责显示，不提供写入 -->
            <Badge
              v-if="props.chapter.done"
              variant="success"
              class="text-[10px] font-normal"
            >
              已学
            </Badge>
            <span
              v-if="kindLabel"
              class="ml-2 align-middle text-xs font-normal text-muted-foreground"
            >
              {{ kindLabel }}
            </span>
          </h4>
          <p
            v-if="props.chapter.summary"
            class="mt-1 text-sm leading-relaxed text-muted-foreground"
          >
            {{ props.chapter.summary }}
          </p>
        </div>
      </button>
      <div
        v-if="statusBadge || canRegenerate || (unreadNewStyle && props.planId)"
        class="flex shrink-0 items-center gap-2"
      >
        <Badge v-if="statusBadge" :variant="statusBadge.variant">
          {{ statusBadge.label }}
        </Badge>
        <!-- 未研读（大纲-only 正常态）：去研读页按需生成 -->
        <Button
          v-if="unreadNewStyle && props.planId"
          variant="outline"
          size="sm"
          data-study-goto
          @click="goStudy"
        >
          <BookOpenText class="h-4 w-4" aria-hidden="true" />
          去研读
        </Button>
        <Button
          v-if="canRegenerate || chapterRegenerating"
          variant="outline"
          size="sm"
          :disabled="
            ((planRegenInFlight || props.regenLocked) &&
              !chapterRegenerating) ||
            chapterRegenerating
          "
          @click="onRegenerate"
        >
          <span
            v-if="chapterRegenerating"
            class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent"
          />
          {{ chapterRegenerating ? "重新生成中…" : "重新生成此章" }}
        </Button>
      </div>
    </header>
    <!-- 重生成反馈：入库/收尾异常与失败原因（折叠态亦可见） -->
    <p v-if="regenError" class="mt-2 text-xs text-destructive">
      {{ regenError }}
    </p>

    <!-- 正文：markdown 渲染（整章标题行折叠，默认展开） -->
    <div v-show="!collapsed" class="mt-6">
      <!-- 章内小节目录：正文 1500-3000 字，靠滚动定位太慢，给小节一个扁平入口 -->
      <nav
        v-if="headings.length > 2"
        class="mb-8 flex flex-wrap items-center gap-x-2 gap-y-1 border-y border-border/60 py-3"
        aria-label="本章目录"
      >
        <span class="text-xs text-muted-foreground">本章目录</span>
        <button
          v-for="(heading, index) in headings"
          :key="index"
          type="button"
          class="min-h-9 rounded px-2 py-1 text-xs transition-colors hover:bg-accent hover:text-primary focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          :class="
            heading.level === 3
              ? 'text-muted-foreground/80'
              : 'font-medium text-muted-foreground'
          "
          @click="scrollToHeading(index)"
        >
          {{ heading.text }}
        </button>
      </nav>
      <div ref="contentRoot">
        <MarkdownContent
          :content-md="props.chapter.contentMd"
          :empty-hint="
            unreadNewStyle
              ? '本章尚未研读——到「研读」页选中即可生成正文。'
              : undefined
          "
        />
      </div>
      <!-- 过程动画：只有「执行过程不可见」的章才有，最多一段；没找到合适的
           形式时宁可不给，也不要塞一个错误的类比 -->
      <VisualizationPlayer
        v-if="props.chapter.visualizations?.length"
        class="mt-3"
        :visualization="props.chapter.visualizations[0]"
      />
      <!-- 结构化自测题：把「知识点 → 题目」的对应摆出来，便于自查有没有漏 -->
      <ExercisePanel
        v-if="props.chapter.exercises?.length"
        class="mt-3"
        :exercises="props.chapter.exercises"
      />
    </div>
  </section>
</template>
