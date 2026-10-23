<script setup lang="ts">
import {
  computed,
  inject,
  nextTick,
  onUnmounted,
  provide,
  ref,
  shallowRef,
  watch,
  type ShallowRef,
} from "vue";
import {
  ArrowLeft,
  ArrowUp,
  Map,
  RotateCcw,
  TriangleAlert,
} from "lucide-vue-next";
import {
  normalizeError,
  regeneratePlan,
  useAgent,
} from "@/composables/useAgent";
import {
  completedPhaseCount,
  ensureProgressListener,
  followedObservation,
  followedPlanId,
  followPlan,
  isObservationTerminal,
  terminalPlanIds,
  unfollowPlan,
} from "@/composables/useGenerationProgress";
import { fetchPlanRecord } from "@/composables/usePlanHistory";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import ClarificationPanel from "@/components/ClarificationPanel.vue";
import PlanForm from "@/components/PlanForm.vue";
import PlanListView from "@/components/PlanListView.vue";
import PlanResult from "@/components/PlanResult.vue";
import LoadingState from "@/components/LoadingState.vue";
import type { PlanRecord, PlanRequest, StudyPlan } from "@/types/agent";

/**
 * PlanWorkbench：学习计划页工作台（从 App.vue 抽出）。
 * - 列表视图（默认）与生成表单/结果视图共用此组件
 * - 生成流状态链（result / 进度观测 / 终态接管）一并随迁，保持
 *   useAgent() 单调用点（每次调用产出全新 refs，app 内只允许一处）
 * - 悬浮层（回顶 / 两级目录）与滚动工具同层承载；滚动宿主经
 *   planScrollHost 注入（App 的 <main> overflow 容器持有真实元素）
 *
 * 根容器刻意不加任何 animation/transform 类：底部悬浮件是 position:fixed，
 * 任何残留 transform 都会成为 containing block 把悬浮件钉进内容盒
 * （目录修复事件的教训）。
 */
const { loading, error, result, generatePlan } = useAgent();

/** 学习计划页内部视图：列表（默认） / 生成表单与结果 */
const planView = ref<"list" | "create">("list");
/** 历史查看模式：主轴展示已保存计划，隐藏参数卡 */
const historyMode = ref(false);
/** 整计划重生成提交中（命令本身很快返回，仅用于按钮防连点） */
const replayingPlan = ref(false);
/**
 * 结果里已有的阶段数：整计划重生成按钮的显示判据。
 * 只有**大纲都没生成出来**（零阶段）才给这个入口——有阶段说明大纲已成功，
 * 该走阶段级/章节级重生成；正在生成（generating）更不该给，否则会把
 * 进行中的任务清空重来。
 */
const resultPhaseCount = computed(() =>
  result.value?.status === "plan" ? result.value.plan.phases.length : 0,
);
/** 历史详情滚动宿主（App 的 <main>，回顶与目录跳转共用） */
const planScrollHost = inject<ShallowRef<HTMLElement | null>>(
  "planScrollHost",
  shallowRef<HTMLElement | null>(null),
);

/** 回到顶部：平滑滚动详情主区 */
function scrollPlanToTop() {
  planScrollHost?.value?.scrollTo({ top: 0, behavior: "smooth" });
}

/** 返回列表（退出查看视图时同步复位历史查看态） */
function exitToPlanList() {
  planView.value = "list";
  historyMode.value = false;
}

/** 目录跳转（悬浮两级目录，与回顶同层 fixed）：平滑定位到阶段/章节锚点 */
function scrollToPhase(index: number) {
  document
    .getElementById(`plan-phase-${index}`)
    ?.scrollIntoView({ behavior: "smooth", block: "start" });
}

/** 两级目录章节跳转：章节锚点包装层在 PhaseCard 内 */
function scrollToChapter(phaseIndex: number, chapterIndex: number) {
  document
    .getElementById(`plan-phase-${phaseIndex}-chapter-${chapterIndex}`)
    ?.scrollIntoView({ behavior: "smooth", block: "start" });
}

/* 目录 → 正文 联动命令信箱：目录不仅要跳转，还要控制正文的展开/折叠
 * （反馈①）。PhaseCard / ChapterSection inject 同一信箱，收到命令后
 * 展开对应层级。滚动与展开命令同批发出（seq 保时序唯一性）。 */
const tocCommands = shallowRef<{
  phase?: number;
  chapter?: { p: number; c: number };
  seq: number;
} | null>(null);
provide("planTocCommands", tocCommands);

/** 目录点阶段：滚动 + 确保该阶段正文展开（PhaseCard 消费） */
function openPhaseFromToc(pi: number) {
  tocCommands.value = { phase: pi, seq: Date.now() };
  scrollToPhase(pi);
}

/** 目录点章节：滚动 + 确保父阶段与章节正文展开（双向联动） */
function openChapterFromToc(pi: number, ci: number) {
  tocCommands.value = { phase: pi, chapter: { p: pi, c: ci }, seq: Date.now() };
  scrollToChapter(pi, ci);
}

/* 滚动监读（scroll-spy）：驱动折叠态「当前阅读阶段」高亮与展开面板的
 * 阶段展开/折叠——两者都跟随外部阅读状态，不在目录内自造独立状态。
 * 取视口前 35% 为判定线（阅读位），最后一个越线锚点即为当前阶段。 */
const activePhase = ref(0);
let spyRaf = 0;

function updateActivePhase() {
  const phases =
    result.value?.status === "plan" ? result.value.plan.phases : [];
  if (!phases.length) {
    activePhase.value = 0;
    return;
  }
  let current = 0;
  for (let i = 0; i < phases.length; i++) {
    const el = document.getElementById(`plan-phase-${i}`);
    if (!el) continue;
    if (el.getBoundingClientRect().top <= window.innerHeight * 0.35) {
      current = i;
    }
  }
  activePhase.value = current;
}

function onHostScroll() {
  if (spyRaf) return;
  spyRaf = requestAnimationFrame(() => {
    spyRaf = 0;
    updateActivePhase();
  });
}

// 目录面板条目的展开/折叠跟随外部阅读状态：当前阶段的章节自动展开（其余折叠）
function phaseExpanded(pi: number) {
  return pi === activePhase.value;
}

watch(
  planScrollHost,
  (host, _prev) => {
    _prev?.removeEventListener("scroll", onHostScroll);
    if (host) {
      host.addEventListener("scroll", onHostScroll, { passive: true });
      updateActivePhase();
    }
  },
  { immediate: true },
);

// 进入详情视图（骨架铺完/PlanResult 挂出锚点）后先校准一次，避免首屏停在 0
watch(
  () => result.value?.status,
  async (status) => {
    if (status === "plan") {
      await nextTick();
      updateActivePhase();
    }
  },
);

onUnmounted(() => {
  planScrollHost?.value?.removeEventListener("scroll", onHostScroll);
  if (spyRaf) cancelAnimationFrame(spyRaf);
});

/** 当前打开的历史记录 id（阶段/章节重生成入口依赖；新生成流程无 id） */
const historyPlanId = shallowRef<string | null>(null);
/**
 * 结果页行级生成状态（任务 25）：get_plan 回读的 PlanRecord.status 快照，
 * 经 prop rowStatus 传给 PlanResult 渲染兜底提示条（generating/failed）。
 * 采用快照而非随 result 重新推导：result 只装 record.plan（StudyPlan）
 * 不含行级字段，两者同步维护。
 */
const resultRowStatus = shallowRef<string | null>(null);
/** 终态快照时的入库告警（审查 minor：终态 unfollow 后应继续展示的结果页属性） */
const resultStorageIssue = ref<string | null>(null);
/** 最近一次生成请求（保存到云端时取 goal / level / weeks 元数据） */
const lastRequest = shallowRef<PlanRequest | null>(null);
/**
 * 生成进度观测视图（useGenerationProgress store）：outline 事件铺骨架树、
 * PhaseDone 按阶段索引记账（去重），数据离开页面也不丢（模块级 store）。
 * 生成终态由两个通道判定：phaseDone 集齐 phaseCount / 静默对账补齐。
 */
const genState = followedObservation;

/** 页面正处于「后台生成进行中」（跟随了 plan 且未终态） */
const generatingNow = computed(
  () =>
    !!genState.value &&
    !isObservationTerminal(genState.value) &&
    !genState.value.removed,
);

/** 大纲骨架已就位（outline 事件已到过；骨架树分支条件） */
const outlineReady = computed(
  () => !!genState.value && genState.value.phases.length > 0,
);

/** 生成完成的进度分母（outline 未到时为 0，进度条隐藏） */
const progressTotal = computed(() => genState.value?.outlinePhaseCount ?? 0);
const progressDone = computed(() =>
  genState.value ? completedPhaseCount(genState.value) : 0,
);

// 生成终态拐点（phaseDone 集齐 / 对账集齐 / 行消失）：回读整份计划展示
// 结果页（planId 注入后阶段/章的重生成入口可用），历史列表行不再转圈。
watch(terminalPlanIds, async (ids) => {
  const pid = followedPlanId.value;
  if (!pid || !ids.includes(pid)) return;
  const obs = genState.value;
  if (obs?.removed) {
    unfollowPlan();
    error.value = {
      code: "generation-gone",
      message:
        "该生成任务已结束，但未能产出可用的计划内容（可能信息不足以成计划或行已被清理）。",
      hint: "可调整目标后重新生成，或到历史记录中查看既有计划。",
    };
    return;
  }
  try {
    const record = await fetchPlanRecord(pid);
    historyPlanId.value = pid;
    // 行级 status 随同一份 record 取到（终态收尾已 settle）：failed 零内容
    // 行不会走到这里被误标（removed 分支已先行拦截），complete 不加标
    resultRowStatus.value = record.status;
    result.value = {
      status: "plan",
      plan: record.plan,
      clarifications: null,
      error: null,
    };
    // 审查 minor：终态 unfollow 会清空 genState，结果页的入库告警徽标若
    // 继续绑 genState?.storageIssue 会失真为「已自动保存」——终态快照时
    // 把 obs.storageIssue 固化到独立 ref，经模板桥接给结果页
    resultStorageIssue.value = obs?.storageIssue ?? null;
  } catch (e) {
    error.value = normalizeError(e);
  } finally {
    unfollowPlan();
  }
});

/**
 * 整计划重生成：清空旧内容后重走完整生成链（连大纲一起重做）。
 *
 * 覆盖场景：大纲阶段就失败 / 收尾没落地而卡在 generating 的计划——这两种
 * 情况下库里没有可点的阶段与章，阶段级重生成按钮永远不会出现。
 */
async function onRegeneratePlan() {
  const pid = historyPlanId.value;
  if (!pid || replayingPlan.value) return;
  const ok = window.confirm(
    "重新生成会清空这份计划现有的阶段与章节，从头编排整份路线。继续吗？",
  );
  if (!ok) return;
  replayingPlan.value = true;
  try {
    await regeneratePlan(pid);
    // 回到生成视图跟随同一 planId：库里已重置为 generating，旧内容不再展示
    result.value = null;
    resultRowStatus.value = null;
    resultStorageIssue.value = null;
    historyMode.value = false;
    historyPlanId.value = pid;
    lastRequest.value = null;
    followPlan(pid);
  } catch (e) {
    error.value = normalizeError(e);
  } finally {
    replayingPlan.value = false;
  }
}

/** 点「新建计划」：切到生成视图（取消跟随旧生成，后台任务继续跑） */
function onCreate() {
  historyMode.value = false;
  historyPlanId.value = null;
  lastRequest.value = null;
  result.value = null;
  resultRowStatus.value = null;
  error.value = null;
  unfollowPlan();
  planView.value = "create";
}

/**
 * 表单提交 → 注册监听 → invoke generate_plan（提交即返 {planId}）。
 * 进度经全局事件驱动 store；终态由 watch(terminalPlanIds) 接管结果页。
 */
async function onSubmit(request: PlanRequest) {
  lastRequest.value = request;
  historyMode.value = false;
  historyPlanId.value = null;
  error.value = null;
  // 时序约定：listen 必须先于 invoke——第一段 outline 事件在命令
  // 返回前后就可能到达，晚注册会丢首骨架
  await ensureProgressListener();
  const submitted = await generatePlan(request);
  if (!submitted) return; // 提交失败：error 已置值，错误卡片展示
  // 参数卡收起为只读摘要条（单轴交互：焦点移交生成过程区）
  followPlan(submitted.planId);
}

/** 查看历史计划：注入已保存的计划并切到查看视图（携带 id 供重生成入口） */
/** 跨页打开详情的拉取中占位（乐观切视图后、详情数据到达前） */
const openingDetail = ref(false);

/**
 * 跨页导航入口（日历抽屉「打开计划详情」）：按计划 id 拉详情后打开。
 * 乐观先切详情布局（historyMode 下表单/页头全隐藏），拉取期间以
 * LoadingState 占位——避免先闪一帧列表再跳详情；拉取失败静默落入
 * error 态由页面呈现。
 */
async function openPlanById(planId: string) {
  openingDetail.value = true;
  planView.value = "create";
  historyMode.value = true;
  historyPlanId.value = planId;
  result.value = null; // 清上次打开残留，防误显旧计划
  error.value = null;
  try {
    const record = await fetchPlanRecord(planId);
    openHistoryPlan(record);
  } catch (e) {
    error.value = normalizeError(e);
  } finally {
    openingDetail.value = false;
  }
}

defineExpose({ openPlanById });

function openHistoryPlan(record: PlanRecord) {
  historyMode.value = true;
  historyPlanId.value = record.id;
  lastRequest.value = null;
  error.value = null;
  // 生成中的历史行重新跟随（反馈②）：全局监听仍活着，恢复 follow 后章节
  // 完成事件重新入 store，对账 watch 持续刷新详情；中途打开无 WS replay，
  // 骨架卡不重建（依赖 outline 事件的 observations 为空），由现有内容 +
  // 生成中提示条主导展示
  if (record.status === "generating") {
    followPlan(record.id);
  } else {
    unfollowPlan();
  }
  // 行级 status 与整份 plan 平级同批回传（PlanRecord 平铺结构，非 record.plan
  // 内容）：generating/failed 详情兜底提示条据此渲染（任务 25）
  resultRowStatus.value = record.status;
  result.value = {
    status: "plan",
    plan: record.plan,
    clarifications: null,
    error: null,
  };
  planView.value = "create";
}

/** 重试：清空错误后由用户重新提交 */
function dismissError() {
  error.value = null;
}

/** 清空生成失败的业务 error 包络（骨架保留，便于看到部分成果与失败标记） */
function dismissResultError() {
  result.value = null;
  resultRowStatus.value = null;
}

/* 生成中详情的实时对账（反馈②展开）：历史打开的 generating 行跟随后续
 * 事件后章/阶段状态在 store 里增量变化；细节正文（PlanResult）不消费
 * store，靠对账 fetchPlanRecord 整体刷新。仅在「本计划正在查看」时触发；
 * 800ms 去抖（章级事件密集不刷屏）；终态由 terminalPlanIds watch 统一接管 */
const midReconcileSignature = computed(() => {
  const obs = genState.value;
  if (!obs) return "";
  return `${Object.keys(obs.phaseStatuses).length}:${
    Object.keys(obs.chapterDone ?? {}).length
  }:${Object.values(obs.phaseStatuses).filter((s) => s === "complete").length}`;
});
let reconcileTimer: ReturnType<typeof setTimeout> | null = null;
watch(midReconcileSignature, async () => {
  const pid = historyPlanId.value;
  if (
    !pid ||
    pid !== followedPlanId.value ||
    !historyMode.value ||
    !genState.value ||
    isObservationTerminal(genState.value)
  ) {
    return;
  }
  if (reconcileTimer) clearTimeout(reconcileTimer);
  reconcileTimer = setTimeout(async () => {
    reconcileTimer = null;
    if (historyPlanId.value !== pid) return; // 已切走别处，丢弃
    try {
      const record = await fetchPlanRecord(pid);
      resultRowStatus.value = record.status;
      if (result.value?.status === "plan") {
        result.value = {
          ...result.value,
          plan: record.plan,
        };
      }
    } catch {
      // 对账失败不打断展示，下个事件再试
    }
  }, 800);
});

/** 阶段/章节重生成完成：用刷新后的计划行整体替换当前结果 */
function onPlanRefreshed(plan: StudyPlan) {
  result.value = {
    status: "plan",
    plan,
    clarifications: null,
    error: null,
  };
}
</script>

<template>
  <!-- 学习计划页工作台：列表（默认） / 生成与查看 -->
  <div class="min-w-0">
    <PlanListView
      v-if="planView === 'list'"
      @create="onCreate"
      @open="openHistoryPlan"
    />

    <div v-else class="mx-auto w-full max-w-6xl px-6 py-8 2xl:max-w-7xl">
      <!-- 返回列表：跨两栏顶部（反馈⑥改位，grid 外层）；sticky 不随滚动；
           反馈⑤：加大字号与圆角胶囊、叠压时阴影/描边/毛玻璃更明显 -->
      <div v-if="planView === 'create'" class="-mt-2 mb-3">
        <Button
          variant="outline"
          class="rounded-full border-border/70 bg-background/90 text-sm shadow-md backdrop-blur transition-colors hover:border-primary/40 hover:bg-muted hover:text-primary"
          @click="exitToPlanList"
        >
          <ArrowLeft class="h-4 w-4" aria-hidden="true" />
          返回列表
        </Button>
      </div>
      <!-- 整计划重生成：仅「大纲未产出」（failed 且零阶段）时提供入口 -->
      <div
        v-if="historyMode && resultRowStatus === 'failed' && resultPhaseCount === 0"
        class="mx-auto mb-4 w-full max-w-3xl rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3"
      >
        <p class="text-sm text-muted-foreground">
          这份计划在第一步（生成大纲）就失败了，库里没有任何阶段与章节，
          所以没有可点的阶段级重生成按钮。整计划重生成会从头编排整份路线。
        </p>
        <Button
          variant="outline"
          size="sm"
          class="mt-2"
          :disabled="replayingPlan"
          @click="onRegeneratePlan"
        >
          <RotateCcw class="h-4 w-4" aria-hidden="true" />
          {{ replayingPlan ? "正在提交…" : "重新生成整个计划" }}
        </Button>
      </div>
      <!-- 交互重构：单轴纵向流——参数卡在顶部（生成中收起为摘要条），
           其下依次过程卡/骨架树/结果；左右双栏并排取消，自上而下一条主轴 -->
      <div class="mx-auto w-full max-w-3xl space-y-5">
        <!-- 跨页打开详情：数据到达前的占位（乐观切视图防列表闪现） -->
        <LoadingState v-if="openingDetail" text="正在打开计划…" />
        <!-- 创建态页头（历史详情由 PlanResult hero 承担，不重复） -->
        <div v-if="!historyMode">
          <h2 class="font-display text-3xl font-semibold tracking-tight">
            生成学习计划
          </h2>
          <p class="mt-1 text-sm text-muted-foreground">
            填好参数，Agent 为你编排分阶段路线，逐章生成并可随时补齐
          </p>
          <div class="mt-4 h-px bg-primary/15" role="presentation" />
        </div>
        <!-- 参数卡：历史详情隐藏；提交后收起为只读摘要条 -->
        <div v-if="!historyMode && !generatingNow">
          <PlanForm :loading="loading" @submit="onSubmit" />
        </div>
        <!-- 生成中的参数摘要条：只读展示（参数在提交后即封板，不提供修改入口） -->
        <div
          v-else-if="!historyMode && generatingNow"
          class="rounded-lg border bg-secondary/40 px-4 py-3"
        >
          <p class="min-w-0 truncate text-sm">
            <span class="font-semibold">参数　</span>
            <span class="text-muted-foreground"
              >{{ lastRequest?.goal }} · {{ lastRequest?.weeklyHours }}h/周 ·
              {{ lastRequest?.weeks }}周</span
            >
          </p>
        </div>
        <section class="min-w-0 space-y-4">
          <!-- 后台生成进行中（提交即返后的过程页：outline 铺骨架，phaseDone 推进；
               task-F: 骨架就绪后本卡退场——继续转圈属反词; 右栏首卡内边距
               p-6 与左侧 PlanForm 卡 p-6 基准一致（task-F 对齐） -->
          <Card v-if="generatingNow && !outlineReady">
            <CardContent
              class="flex flex-col items-center gap-4 p-6 text-center"
            >
              <div
                class="h-10 w-10 animate-spin rounded-full border-2 border-primary border-t-transparent"
              />
              <div>
                <p class="font-medium">Agent 正在编排你的学习计划…</p>
                <p class="mt-1 text-sm text-muted-foreground">
                  后台生成，可随时离开页面，进度不会中断
                </p>
              </div>
              <!-- 云端入库异常提示（storageIssue 事件，不中断生成） -->
              <p
                v-if="genState?.storageIssue"
                class="break-all text-xs text-destructive"
              >
                <TriangleAlert
                  class="mr-1 inline h-3.5 w-3.5 align-[-2px]"
                  aria-hidden="true"
                />{{ genState.storageIssue }}
              </p>
              <!-- 生成进度：第 N/M 阶段完成 + 进度条（outline 未到则隐藏） -->
              <div v-if="progressTotal" class="w-full max-w-xs">
                <div
                  class="mb-1.5 flex justify-between text-xs text-muted-foreground"
                >
                  <span
                    >第 {{ progressDone }}/{{ progressTotal }} 阶段完成</span
                  >
                  <span
                    >{{
                      Math.round(
                        (progressDone / Math.max(progressTotal, 1)) * 100,
                      )
                    }}%</span
                  >
                </div>
                <div class="h-1.5 overflow-hidden rounded-full bg-muted">
                  <div
                    class="h-full rounded-full bg-primary transition-[width] duration-500"
                    :style="{
                      width: `${(progressDone / Math.max(progressTotal, 1)) * 100}%`,
                    }"
                  />
                </div>
              </div>
            </CardContent>
          </Card>

          <!-- 提交在飞窗口（极短；generating 卡片就位前占位） -->
          <Card v-else-if="loading">
            <CardContent
              class="flex flex-col items-center gap-4 py-14 text-center"
            >
              <div
                class="h-10 w-10 animate-spin rounded-full border-2 border-primary border-t-transparent"
              />
              <p class="text-sm text-muted-foreground">正在提交生成任务…</p>
            </CardContent>
          </Card>

          <!-- 错误 -->
          <Card v-else-if="error" class="border-destructive/40">
            <CardContent class="space-y-3 py-8 text-center">
              <TriangleAlert
                class="h-7 w-7 text-destructive"
                aria-hidden="true"
              />
              <div>
                <p class="font-semibold text-destructive">
                  请求失败（{{ error.code }}）
                </p>
                <p class="mt-1 break-all text-sm text-muted-foreground">
                  {{ error.message }}
                </p>
                <p v-if="error.hint" class="mt-2 text-sm">
                  💡 {{ error.hint }}
                </p>
              </div>
              <!-- 骨架已铺开说明大纲已产出：部分成果已入库，引导去历史记录重生成 -->
              <div v-if="genState?.phases.length" class="text-sm">
                <p class="text-muted-foreground">
                  已完成的阶段已保存在云端，可到历史记录中查看并重新生成失败部分。
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  class="mt-2"
                  @click="planView = 'list'"
                  >查看历史记录</Button
                >
              </div>
              <Button variant="outline" size="sm" @click="dismissError"
                >我知道了</Button
              >
            </CardContent>
          </Card>

          <!-- 生成失败的业务 error 包络（HTTP 200 + status=error） -->
          <Card
            v-else-if="result?.status === 'error'"
            class="border-destructive/40"
          >
            <CardContent class="space-y-3 py-8 text-center">
              <TriangleAlert
                class="h-7 w-7 text-destructive"
                aria-hidden="true"
              />
              <div>
                <p class="font-semibold text-destructive">
                  生成失败（{{ result.error.code }}）
                </p>
                <p class="mt-1 break-all text-sm text-muted-foreground">
                  {{ result.error.message }}
                </p>
              </div>
              <!-- 骨架已铺开说明大纲已产出：部分成果已入库，引导去历史记录重生成 -->
              <div v-if="genState?.phases.length" class="text-sm">
                <p class="text-muted-foreground">
                  已完成的阶段已保存在云端，可到历史记录中查看并重新生成失败部分。
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  class="mt-2"
                  @click="planView = 'list'"
                  >查看历史记录</Button
                >
              </div>
              <Button variant="outline" size="sm" @click="dismissResultError"
                >我知道了</Button
              >
            </CardContent>
          </Card>

          <!-- 澄清问题 -->
          <ClarificationPanel
            v-else-if="result?.status === 'clarification'"
            :questions="result.clarifications"
          />

          <!-- 计划结果 -->
          <PlanResult
            v-else-if="result?.status === 'plan'"
            :plan="result.plan"
            :plan-id="historyPlanId ?? undefined"
            :goal="lastRequest?.goal"
            :level="lastRequest?.level"
            :weeks="lastRequest?.weeks"
            :history-mode="historyMode"
            :storage-issue="genState?.storageIssue"
            :row-status="resultRowStatus ?? undefined"
            @refreshed="onPlanRefreshed"
          />

          <!-- 空态（跨页打开详情的加载窗口由顶部 LoadingState 承担，此处不闪） -->
          <Card v-else-if="!openingDetail" class="border-dashed">
            <CardContent
              class="flex flex-col items-center gap-2 p-6 text-center"
            >
              <Map class="h-9 w-9 text-primary/50" aria-hidden="true" />
              <p class="font-medium">你的学习路线图将在这里出现</p>
              <p class="max-w-sm text-sm text-muted-foreground">
                填好上方参数卡后点击「生成学习计划」，Agent
                会先检查信息是否完整，再生成、评审并输出分阶段计划。
              </p>
            </CardContent>
          </Card>

          <!-- 大纲骨架树（渐进式渲染，骨架是过程态：终态后由 PlanResult 接管） -->
          <Card v-if="outlineReady && !result" aria-live="polite">
            <!-- task-C: p-6 使首内边距与左 PlanForm 卡 CardHeader 的 p-6 对齐
                 （默认 CardContent 是 p-6 pt-0，首元素贴卡顶，曾导致两栏顶部不齐） -->
            <CardContent class="space-y-4 p-6">
              <div>
                <h3 class="text-sm font-semibold">
                  {{ genState?.title }}
                </h3>
                <p
                  v-if="genState?.overview"
                  class="mt-1 text-xs leading-relaxed text-muted-foreground"
                >
                  {{ genState.overview }}
                </p>
              </div>
              <div class="space-y-3">
                <div
                  v-for="(phase, i) in genState?.phases"
                  :key="i"
                  class="rounded-lg border p-3"
                >
                  <div class="flex items-center justify-between gap-2">
                    <span class="text-sm font-medium">{{ phase.name }}</span>
                    <div class="flex items-center gap-2">
                      <span
                        v-if="phase.weekRange"
                        class="min-w-16 text-right text-xs tabular-nums text-muted-foreground"
                        >{{ phase.weekRange }}</span
                      >
                      <!-- 阶段位：complete 绿 / failed 红 / 进行中不标
                           （无真实失败信号不标红，storageIssue 走提示） -->
                      <Badge
                        v-if="genState?.phaseStatuses[i] === 'complete'"
                        variant="success"
                        >✓ 已完成</Badge
                      >
                      <Badge
                        v-else-if="genState?.phaseStatuses[i] === 'failed'"
                        variant="destructive"
                        >✗ 失败</Badge
                      >
                    </div>
                  </div>
                  <ul v-if="phase.chapterTitles.length" class="mt-2 space-y-1">
                    <li
                      v-for="(chapterTitle, ci) in phase.chapterTitles"
                      :key="ci"
                      class="flex items-center gap-2 text-sm text-muted-foreground"
                    >
                      <!-- 章节点：chapterDone 事件点亮章级真完成（emerald，
                           task-D2）；阶段整段完成同为绿；failed 红；进行中灰 -->
                      <span
                        class="h-1.5 w-1.5 shrink-0 rounded-full"
                        :class="
                          genState?.chapterDone?.[`${i}:${ci}`] ||
                          genState?.phaseStatuses[i] === 'complete'
                            ? 'bg-emerald-500'
                            : genState?.phaseStatuses[i] === 'failed'
                              ? 'bg-destructive'
                              : 'bg-muted-foreground/40'
                        "
                      /><span class="truncate"
                        >{{ chapterTitle
                        }}{{
                          genState?.chapterDone?.[`${i}:${ci}`] ||
                          genState?.phaseStatuses[i] === "complete"
                            ? ""
                            : "（待生成）"
                        }}</span
                      >
                    </li>
                  </ul>
                </div>
              </div>
            </CardContent>
          </Card>
        </section>
      </div>
    </div>

    <!-- 详情页回顶小按钮（task-F：fixed 右下角，历史详情模式下显示） -->
    <Button
      v-if="historyMode"
      variant="outline"
      size="sm"
      class="fixed bottom-6 right-8 z-30 h-9 w-9 rounded-full p-0 text-muted-foreground transition-colors hover:border-primary/40 hover:bg-muted hover:text-primary"
      v-tip="'回到顶部'"
      aria-label="回到顶部"
      @click="scrollPlanToTop"
    >
      <ArrowUp class="h-4 w-4" aria-hidden="true" />
    </Button>

    <!-- 悬浮两级目录（与回顶同层 fixed）：折叠态不再是一整根竖条，而是每个
         阶段一小段（右缘留距更宽松）；当前阅读阶段的短条用主题色加长高亮，
         折叠态可直接点击任意短条跳阶段。悬浮展开完整面板，面板内章节条目
         跟随外部阅读状态：当前阶段自动展开、其余折叠（滚动监读驱动，目录
         内不自选状态）；本组件根无 transform，fixed 恒等于视口定位 -->
    <div
      v-if="planView === 'create' && result?.status === 'plan'"
      class="group fixed right-3 top-1/2 z-30 -translate-y-1/2 select-none py-6 pr-1"
    >
      <!-- 折叠态：每阶段一小条；hover 面板遮计时保持整组可触发 -->
      <div class="flex flex-col items-end gap-1.5">
        <button
          v-for="(phase, pi) in result.plan.phases"
          :key="pi"
          class="w-4 rounded-full transition-[height,background-color] duration-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :class="
            pi === activePhase
              ? 'h-2.5 bg-primary group-hover:bg-primary/60'
              : 'h-1.5 bg-border/70 group-hover:bg-primary/30'
          "
          v-tip="`${pi + 1}. ${phase.name}`"
          :aria-label="`跳转到阶段 ${pi + 1}：${phase.name}`"
          @click="openPhaseFromToc(pi)"
        />
      </div>
      <nav
        class="invisible absolute right-0 top-1/2 max-h-[65vh] w-56 -translate-y-1/2 overflow-y-auto rounded-xl border bg-card/95 p-2.5 opacity-0 shadow-lg backdrop-blur transition-opacity duration-200 group-hover:visible group-hover:opacity-100"
        aria-label="计划目录"
      >
        <div
          v-for="(phase, pi) in result.plan.phases"
          :key="pi"
          class="mb-2 last:mb-0"
        >
          <button
            class="w-full truncate rounded-lg px-2 py-1.5 text-left text-xs font-semibold transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :class="pi === activePhase ? 'text-primary' : ''"
            v-tip="'跳转并展开该阶段'"
            @click="openPhaseFromToc(pi)"
          >
            {{ pi + 1 }}. {{ phase.name }}
          </button>
          <!-- 章节条目跟随阅读状态：仅当前阅读阶段展开，其余折叠 -->
          <div v-if="phaseExpanded(pi)" class="ml-4 space-y-0.5 border-l pl-2">
            <button
              v-for="(chapter, ci) in phase.chapters || []"
              :key="ci"
              class="w-full truncate rounded-lg px-2 py-1 text-left text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              v-tip="chapter.title"
              @click="openChapterFromToc(pi, ci)"
            >
              {{ pi + 1 }}.{{ ci + 1 }} {{ chapter.title }}
            </button>
          </div>
        </div>
      </nav>
    </div>
  </div>
</template>
