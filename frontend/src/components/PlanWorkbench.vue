<script setup lang="ts">
import {
  computed,
  inject,
  nextTick,
  onMounted,
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
  List,
  Maximize2,
  Minimize2,
  X,
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
import { readerStickyOffset, scrollToWithinMain } from "@/lib/utils";
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
const readerFocused = inject("readerFocused", ref(false));
const tocOpen = ref(false);
/** 目录面板（打开后焦点移入，Esc 与键盘导航随即可用） */
const tocPanelEl = ref<HTMLElement | null>(null);
const storedFontSize = Number(localStorage.getItem("praxis.readerFontSize"));
const readerFontSize = ref(
  [16, 17, 18, 19, 20].includes(storedFontSize) ? storedFontSize : 17,
);
watch(readerFontSize, (size) =>
  localStorage.setItem("praxis.readerFontSize", String(size)),
);
onUnmounted(() => {
  readerFocused.value = false;
});
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

/** 阅读锚点：视口前 35% 线之上最后一个结构元素（章锚点或章内标题） */
function findReadingAnchor(): HTMLElement | null {
  const candidates = planScrollHost?.value?.querySelectorAll<HTMLElement>(
    '[id^="plan-phase-"], .markdown-code h2, .markdown-code h3',
  );
  if (!candidates) return null;
  let anchor: HTMLElement | null = null;
  for (const el of candidates) {
    if (el.getBoundingClientRect().top <= window.innerHeight * 0.35)
      anchor = el;
  }
  return anchor;
}

/**
 * 排版突变（字号/专注态切换）前后保持阅读位置：记录锚点元素突变前的
 * 视口偏移，突变应用后把差值补回滚动宿主——重排改变行高/行宽时，读者
 * 正在读的那一行保持在原地，而不是跳到别处。
 */
async function withReadingPosition(apply: () => void) {
  const anchor = findReadingAnchor();
  const before = anchor?.getBoundingClientRect().top;
  apply();
  await nextTick();
  if (!anchor || before === undefined) return;
  const delta = anchor.getBoundingClientRect().top - before;
  if (delta) {
    planScrollHost?.value?.scrollBy({ top: delta, behavior: "instant" });
  }
}

/** 字号切换：正文整体重排，锚点补偿防止阅读位置漂移（watch 在重排前同步记位） */
watch(readerFontSize, async (size, prev) => {
  if (size === prev) return;
  await withReadingPosition(() => {});
});

/** 专注模式切换：隐藏侧栏/顶栏改变内容宽度，同样锚点补偿 */
async function toggleReaderFocus() {
  await withReadingPosition(() => {
    readerFocused.value = !readerFocused.value;
  });
  if (!readerFocused.value) {
    // Escape/按钮退出后把焦点还给触发按钮，键盘用户不落回 body
    await nextTick();
    document
      .querySelector<HTMLButtonElement>('[aria-label="专注阅读"]')
      ?.focus();
  }
}

/** 打开目录：面板挂载后把焦点移入（Esc 与键盘导航随即可用） */
async function openToc() {
  tocOpen.value = true;
  await nextTick();
  tocPanelEl.value?.focus();
}

/** 关闭目录：焦点返回触发按钮（经 aria-controls 定位，键盘用户不落在 body） */
async function closeToc() {
  tocOpen.value = false;
  await nextTick();
  document
    .querySelector<HTMLButtonElement>('[aria-controls="reader-toc"]')
    ?.focus();
}

/** 回到顶部：平滑滚动详情主区 */
function scrollPlanToTop() {
  planScrollHost?.value?.scrollTo({
    top: 0,
    behavior: matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "instant"
      : "smooth",
  });
}

/** 返回列表（退出查看视图时同步复位历史查看态） */
function exitToPlanList() {
  planView.value = "list";
  historyMode.value = false;
  readerFocused.value = false;
  tocOpen.value = false;
  resultRevealPending.value = false;
}


/** 目录跳转（悬浮两级目录，与回顶同层 fixed）：只在 main 内定位，避免顶栏被根滚动顶出 */
function scrollToPhase(index: number) {
  scrollToWithinMain(
    document.getElementById(`plan-phase-${index}`),
    readerStickyOffset(36),
  );
}

/** 两级目录章节跳转：章节锚点包装层在 PhaseCard 内（scroll-mt-24 = 96px） */
function scrollToChapter(phaseIndex: number, chapterIndex: number) {
  scrollToWithinMain(
    document.getElementById(`plan-phase-${phaseIndex}-chapter-${chapterIndex}`),
    readerStickyOffset(12),
  );
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
async function openPhaseFromToc(pi: number) {
  // 点击即高亮：目标行可能因懒渲染后的页面高度变化落在 35% 判定线下方，
  // 只靠滚动监读会让目录短暂指回上一阶段；用户主动选择优先。
  activePhase.value = pi;
  tocJumpGuardUntil = Date.now() + 900;
  tocCommands.value = { phase: pi, seq: Date.now() };
  await nextTick();
  scrollToPhase(pi);
}

/** 目录点章节：滚动 + 确保父阶段与章节正文展开（双向联动） */
async function openChapterFromToc(pi: number, ci: number) {
  activePhase.value = pi;
  tocJumpGuardUntil = Date.now() + 900;
  tocCommands.value = { phase: pi, chapter: { p: pi, c: ci }, seq: Date.now() };
  await nextTick();
  scrollToChapter(pi, ci);
}

/* 滚动监读（scroll-spy）：驱动折叠态「当前阅读阶段」高亮与展开面板的
 * 阶段展开/折叠——两者都跟随外部阅读状态，不在目录内自造独立状态。
 * 取视口前 35% 为判定线（阅读位），最后一个越线锚点即为当前阶段。 */
const activePhase = ref(0);
/** 目录主动跳转后的短暂抑制窗口（见 openPhaseFromToc 注释） */
let tocJumpGuardUntil = 0;
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
  if (Date.now() < tocJumpGuardUntil) return;
  if (spyRaf) return;
  spyRaf = requestAnimationFrame(() => {
    spyRaf = 0;
    if (Date.now() < tocJumpGuardUntil) return;
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

/**
 * 全局 Escape：目录开着先关目录（焦点回触发按钮），否则专注态退出专注。
 * 输入控件内的 Escape 留给控件自身（取消重命名/收起原生下拉）；组合输入
 * （IME）过程中的 Escape 不作处理。目录面板自身的 Esc 处理会 stop 传播，
 * 不会走到这里重复关闭。
 */
function onWindowKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape" || event.isComposing) return;
  const target = event.target as HTMLElement | null;
  if (target?.closest("input, textarea, select")) return;
  if (tocOpen.value) {
    event.preventDefault();
    void closeToc();
  } else if (readerFocused.value) {
    event.preventDefault();
    void toggleReaderFocus();
  }
}

onMounted(() => window.addEventListener("keydown", onWindowKeydown));

onUnmounted(() => {
  window.removeEventListener("keydown", onWindowKeydown);
  planScrollHost?.value?.removeEventListener("scroll", onHostScroll);
  if (spyRaf) cancelAnimationFrame(spyRaf);
  // 生成中对账定时器随组件卸载取消（闭包内还会比对 planId，双保险）
  if (reconcileTimer) clearTimeout(reconcileTimer);
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
/** 本次提交是否等待结果到达反馈（只聚焦一次，历史查看不抢焦点） */
const resultRevealPending = ref(false);
/** 生成过程/结果的读屏状态播报 */
const liveMessage = ref("");
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
      hint: "可调整目标后重新生成，或回到列表查看既有计划。",
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
  // 到达反馈必须在 unfollow 之后再滚：generatingNow→false 会让参数表单
  // 重新挂载、结果区位置此时才稳定；在 try 内提前滚动，落点会被表单回插
  // 推偏（评审复验实测视口停在表单顶部）。removed/异常分支已在上方
  // return/由错误卡接管，不会走到这里。
  await nextTick();
  scrollToWithinMain(
    document.getElementById("plan-result-top"),
    readerStickyOffset(16),
  );
  if (resultRevealPending.value && planView.value === "create") {
    liveMessage.value = "学习计划已生成，已定位到结果标题";
    await nextTick();
    document
      .getElementById("plan-result-title")
      ?.focus({ preventScroll: true });
  }
  resultRevealPending.value = false;
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
  if (!submitted) {
    // 提交失败：error 已置值，保留表单与输入，错误卡片在下方展示
    liveMessage.value = "生成请求未提交，请查看错误信息";
    return;
  }
  // 参数卡收起为只读摘要条（单轴交互：焦点移交生成过程区）
  resultRevealPending.value = true;
  liveMessage.value = "已提交，正在下方生成学习计划";
  followPlan(submitted.planId);
  // 到达反馈：把生成过程区带入视口（表单仍在原位，首屏无变化会让用户
  // 以为没点上而重复提交）。只在 main 内滚动，尊重 reduced-motion。
  await nextTick();
  scrollToWithinMain(
    document.getElementById("generation-flow"),
    readerStickyOffset(12),
  );
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

defineExpose({ openPlanById, exitToPlanList });

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

    <div
      v-else
      class="mx-auto w-full max-w-[880px] px-5 pb-16 sm:px-10"
      :style="{ '--reader-font-size': `${readerFontSize}px` }"
    >
      <p class="sr-only" role="status" aria-live="polite">
        {{ liveMessage }}
      </p>
      <!-- 整计划重生成：仅「大纲未产出」（failed 且零阶段）时提供入口 -->
      <div
        v-if="
          historyMode && resultRowStatus === 'failed' && resultPhaseCount === 0
        "
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
      <!-- 部分完成：有内容可用，只是缺了几章。刻意与上面的"整计划失败"分开：
           那种情况没有任何内容可看，这里恰恰相反——所以给人的动作是
           "把缺的补上"，而不是"从头再来" -->
      <div
        v-if="historyMode && resultRowStatus === 'partial'"
        class="mx-auto mb-4 w-full max-w-3xl rounded-lg border border-border bg-muted/40 px-4 py-3"
      >
        <p class="text-sm text-muted-foreground">
          这份计划有一部分章节没能生成，已生成的章节都保留着。
          翻到空白的章节点「重新生成」即可单独补齐，不必重做整份计划。
        </p>
      </div>
      <!-- 返回列表工具栏：sticky 吸附滚动宿主顶部（在内容流内自动对齐
           内容区左缘，不压侧栏），bg/backdrop 保证滚动时正文不透出 -->
      <div v-if="planView === 'create'" class="reader-toolbar">
        <Button variant="ghost" size="sm" class="h-10" @click="exitToPlanList">
          <ArrowLeft class="h-4 w-4" aria-hidden="true" />学习计划
        </Button>
        <div
          v-if="result?.status === 'plan'"
          class="flex items-center gap-1 sm:gap-2"
        >
          <label class="flex items-center gap-2 text-xs text-muted-foreground">
            <span class="hidden sm:inline">字号</span>
            <select
              v-model.number="readerFontSize"
              aria-label="正文字号"
              class="h-10 rounded-lg border bg-card px-2 text-sm text-foreground"
            >
              <option
                v-for="size in [16, 17, 18, 19, 20]"
                :key="size"
                :value="size"
              >
                {{ size }} px
              </option>
            </select>
          </label>
          <Button
            variant="ghost"
            size="sm"
            class="h-10"
            :aria-pressed="readerFocused"
            :aria-label="readerFocused ? '退出专注阅读' : '专注阅读'"
            @click="toggleReaderFocus"
          >
            <Minimize2
              v-if="readerFocused"
              class="h-4 w-4"
              aria-hidden="true"
            />
            <Maximize2 v-else class="h-4 w-4" aria-hidden="true" />
            <span class="hidden sm:inline">{{
              readerFocused ? "退出专注" : "专注阅读"
            }}</span>
          </Button>
          <Button
            variant="outline"
            size="sm"
            class="h-10"
            :aria-expanded="tocOpen"
            aria-controls="reader-toc"
            @click="tocOpen ? closeToc() : openToc()"
          >
            <List class="h-4 w-4" aria-hidden="true" />目录
          </Button>
        </div>
      </div>
      <!-- 交互重构：单轴纵向流——参数卡在顶部（生成中收起为摘要条），
           其下依次过程卡/骨架树/结果；左右双栏并排取消，自上而下一条主轴 -->
      <div class="mx-auto w-full max-w-[740px] space-y-5">
        <!-- 跨页打开详情：数据到达前的占位（乐观切视图防列表闪现） -->
        <LoadingState v-if="openingDetail" text="正在打开计划…" />
        <!-- 创建态页头（历史详情由 PlanResult hero 承担，不重复） -->
        <div v-if="!historyMode && !result" class="pb-3">
          <h1 class="page-heading">开启一段新的学习</h1>
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
        <section id="generation-flow" class="min-w-0 space-y-4">
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
              <!-- 骨架已铺开说明大纲已产出：部分成果已入库，引导回列表重生成 -->
              <div v-if="genState?.phases.length" class="text-sm">
                <p class="text-muted-foreground">
                  已完成的阶段已保存在云端，可回到列表打开这份计划，重新生成失败的部分。
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  class="mt-2"
                  @click="planView = 'list'"
                  >返回列表</Button
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
              <!-- 骨架已铺开说明大纲已产出：部分成果已入库，引导回列表重生成 -->
              <div v-if="genState?.phases.length" class="text-sm">
                <p class="text-muted-foreground">
                  已完成的阶段已保存在云端，可回到列表打开这份计划，重新生成失败的部分。
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  class="mt-2"
                  @click="planView = 'list'"
                  >返回列表</Button
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
      v-if="planView === 'create' && result?.status === 'plan' && tocOpen"
      id="reader-toc"
      ref="tocPanelEl"
      role="dialog"
      aria-label="阅读目录"
      tabindex="-1"
      class="fixed bottom-20 right-4 z-30 max-h-[65dvh] w-72 max-w-[calc(100vw-2rem)] overflow-y-auto rounded-2xl border bg-popover p-4 shadow-xl outline-none sm:right-8"
      @keydown.esc.stop.prevent="closeToc"
    >
      <div class="mb-3 flex items-center justify-between border-b pb-3">
        <span class="text-sm font-semibold">阅读目录</span>
        <button
          class="flex h-10 w-10 items-center justify-center rounded-lg hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="关闭目录"
          @click="closeToc"
        >
          <X class="h-4 w-4" />
        </button>
      </div>
      <nav aria-label="计划目录">
        <div
          v-for="(phase, pi) in result.plan.phases"
          :key="pi"
          class="mb-2 last:mb-0"
        >
          <button
            class="min-h-11 w-full rounded-lg px-3 py-2 text-left text-sm font-semibold transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :class="pi === activePhase ? 'bg-accent text-primary' : ''"
            :aria-current="pi === activePhase ? 'location' : undefined"
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
              class="min-h-10 w-full rounded-lg px-2 py-2 text-left text-xs leading-5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
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
