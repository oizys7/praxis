import { computed, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { fetchPlanRecord } from "@/composables/usePlanHistory";
import type { OutlinePhasePayload, PlanProgressEnvelope } from "@/types/agent";

/** 事件信封的本地别名（listen 的 payload 形状 = PlanProgressEnvelope） */
type Envelope = PlanProgressEnvelope;

/**
 * useGenerationProgress：后台生成进度流的全局事件观测 store（任务 23）。
 *
 * generate_plan 已改为提交即返（{planId}），生成由 Python 后台任务执行，
 * 进度经 tauri 全局事件 'plan-progress' 推送（{planId, kind, payload}）。
 * 本模块承担三件事：
 * - 模块级一次性 subscribe：App 挂载/提交前经 ensureProgressListener 注册，
 *   事件写入模块级 reactive store，页面切换 / 组件卸载不影响订阅与数据，
 *   重进页面直接恢复展示（离开页面后生成在后台继续）；
 * - 去重计数：PhaseDone 按「阶段索引状态集合」记账（同 index 重复推送幂等），
 *   修复事件到达计数在重连重放时的假进度（用户反馈①）；
 * - 终态兜底：tauri 事件无终态信号（done 不 emit），以
 *   ①PhaseDone 索引集齐 outline.phaseCount、②静默对账轮询
 *   （90s 无事件后每 30s 拉一次详情，按阶段 status 补齐）双通道判定完成。
 */

/** 单个后台生成任务的观测态（骨架 + 阶段状态集合 + 提示） */
export interface GenerationObservation {
  planId: string;
  /** 大纲骨架（outline 事件优先；详情对账不覆盖——骨架变化只增不改位） */
  title: string;
  overview: string;
  outlinePhaseCount: number;
  phases: OutlinePhasePayload[];
  /** 阶段索引 -> 状态：PhaseDone 记 complete；详情对账可补 failed */
  phaseStatuses: Record<number, "complete" | "failed">;
  /** 最近一次 storageIssue 提示（云端入库异常，不中断生成） */
  storageIssue: string | null;
  /** 最近一次进度事件到达时间戳（静默判定与对账节奏依据） */
  lastEventAt: number;
  /** 上一次对账时间戳（90s 静默后限流为每 30s 一次） */
  lastPollAt: number;
  /** 静默对账取详情连续失败次数（>=3 视为行已消失/不可达，停止追问） */
  missCount: number;
  /** 对账判定该计划行已不存在（被清理/删除，无从展示内容） */
  removed: boolean;
  /** 已判定终态（索引集齐或对账集齐；判定只做一次） */
  terminal: boolean;
}

/** 全局观测表：planId -> 观测态（模块级，页面切换不丢） */
export const observations = ref<Record<string, GenerationObservation>>({});

/** 当前页面跟随展示的计划 id（App.vue 提交成功后设置；新建时清空） */
export const followedPlanId = ref<string | null>(null);

/** 终态信号量：任一观测 plan 判定完成/消失时自增（历史列表刷新信号） */
export const terminalCounter = ref(0);

/** 对账轮询判定「静默」的阈值（进度流 90s 无事件，与规划一致） */
const POLL_SILENCE_MS = 90_000;
/** 静默后的对账节奏（每 30s 拉一次详情） */
const POLL_INTERVAL_MS = 30_000;
/** 轮询器空转的探测周期（远短于判定阈值，保证节奏不准点也无妨） */
const POLL_TICK_MS = 5_000;
/** 连续对账失败次数到该值 → 视为计划行已消失 */
const MISS_LIMIT = 3;

/** 空白观测态（followPlan / 事件到达建槽共用） */
function newObservation(planId: string): GenerationObservation {
  return {
    planId,
    title: "",
    overview: "",
    outlinePhaseCount: 0,
    phases: [],
    phaseStatuses: {},
    storageIssue: null,
    lastEventAt: Date.now(),
    lastPollAt: 0,
    missCount: 0,
    removed: false,
    terminal: false,
  };
}

/** 观测态是否已达终态：大纲已到且 phaseStatuses 覆盖全部阶段索引 */
export function isObservationTerminal(
  obs: GenerationObservation | undefined,
): boolean {
  if (!obs || obs.terminal || obs.removed || obs.outlinePhaseCount <= 0) {
    return false;
  }
  // ruff 无关的 TS 细节：Object.keys 对稀疏 record 的键数即可做覆盖判断，
  // 索引键值均为数字，无需逐位比 Set
  return Object.keys(obs.phaseStatuses).length >= obs.outlinePhaseCount;
}

/** 已判定终态的计划 id 列表（响应式视图，供 watch） */
export const terminalPlanIds = computed(() =>
  Object.values(observations.value)
    .filter((obs) => isObservationTerminal(obs))
    .map((obs) => obs.planId),
);

/** 当前跟随观测态的引用视图（undefined = 页面未跟随任何生成） */
export const followedObservation = computed(() =>
  followedPlanId.value ? observations.value[followedPlanId.value] : undefined,
);

/** 通知终态：置位 + 信号量自增（一次性；重入时直接返回） */
function notifyTerminal(obs: GenerationObservation) {
  if (obs.terminal) return;
  obs.terminal = true;
  terminalCounter.value += 1;
}

/** 单条进度事件的记账（outline 幂等可重放；phaseDone 集合去重） */
function applyProgressEvent(
  obs: GenerationObservation,
  kind: string,
  payload: Envelope["payload"],
) {
  // kind=plan 才是 generate 链路（regen 不走全局事件）；防御未知 kind
  if (kind !== "plan") return;
  obs.lastEventAt = Date.now();
  switch (payload.type) {
    case "outline": {
      // 重连回放可能让 outline 晚到：只重写大纲骨架字段，不触碰已记账状态
      obs.title = payload.title;
      obs.overview = payload.overview ?? "";
      obs.outlinePhaseCount = payload.phaseCount;
      obs.phases = payload.phases ?? [];
      break;
    }
    case "phaseDone": {
      // index 为 0-based 阶段索引，total=outline.phaseCount（generate 链路）；
      // 同 index 重复推送（重连回放）幂等：Set 语义按索引状态记账
      obs.phaseStatuses[payload.index] = "complete";
      break;
    }
    case "storageIssue": {
      obs.storageIssue = payload.message;
      break;
    }
  }
  if (isObservationTerminal(obs)) notifyTerminal(obs);
}

/** 模块级一次性注册：resolve 后 UnlistenFn 存 module 局部（dispose 用） */
let unlisten: UnlistenFn | null = null;
let listenPromise: Promise<void> | null = null;

/**
 * 注册全局 'plan-progress' 监听（幂等，多调用方共享一个 promise）。
 * App.vue 在提交（invoke generate_plan）**之前** await 本函数——
 * 第一段 outline 事件可能在命令返回前后就发出，listen 必须先于 invoke。
 */
export function ensureProgressListener(): Promise<void> {
  if (listenPromise) return listenPromise;
  const own = listen<PlanProgressEnvelope>("plan-progress", (event) => {
    const { planId, kind, payload } = event.payload;
    if (!planId) return;
    // 槽位事件式建槽：applyProgressEvent 内部消费（含晚到的 outline）
    if (!observations.value[planId]) {
      observations.value[planId] = newObservation(planId);
    }
    applyProgressEvent(observations.value[planId], kind, payload);
  })
    .then((fn): void => {
      // 审查 Finding-2：dispose 先于在途 listen 兑现时，监听在卸载后
      // 「复活」且永不再释放——token 校验失效（listenPromise 已换新/清空）
      // 即立即释放，不赋给 unlisten
      if (listenPromise !== own) {
        fn();
        return;
      }
      unlisten = fn;
      startPolling();
    })
    .catch((e) => {
      // 审查 Finding-7：listen 失败（IPC 关停瞬时）不永久缓存被拒 promise，
      // 复位后允许下次调用重试；错误继续上抛给 await 者感知
      if (listenPromise === own) {
        listenPromise = null;
      }
      throw e;
    });
  listenPromise = own;
  return own;
}

/** 解除全局监听并停掉对账轮询（App unmount 时机调用） */
export function disposeProgressListener() {
  stopPolling();
  unlisten?.();
  unlisten = null;
  listenPromise = null;
}

/** 页面跟随某次后台生成（App 提交成功拿到 planId 后调用） */
export function followPlan(planId: string) {
  if (!observations.value[planId]) {
    observations.value[planId] = newObservation(planId);
  }
  followedPlanId.value = planId;
}

/** 取消页面跟随（新建计划等场景；观测与订阅保留，生成不受影响） */
export function unfollowPlan() {
  followedPlanId.value = null;
}

// ---------------------------------------------------------------------------
// 静默对账轮询（终态兜底通道②）：done 不 emit，事件停到不代表生成停下，
// 90s 静默后每 30s 拉一次详情补齐（读 plan row 阶段 status 逐位对账）
// ---------------------------------------------------------------------------

let pollTimer: ReturnType<typeof setInterval> | undefined;

function startPolling() {
  if (pollTimer !== undefined) return;
  pollTimer = setInterval(() => void reconcileDueObservations(), POLL_TICK_MS);
}

function stopPolling() {
  if (pollTimer === undefined) return;
  clearInterval(pollTimer);
  pollTimer = undefined;
}

/** 到期对账：静默超阈值且距上轮 ≥ 30s 的未终态观测，逐一拉详情补齐 */
async function reconcileDueObservations() {
  const now = Date.now();
  for (const obs of Object.values(observations.value)) {
    if (obs.terminal || obs.removed) continue;
    if (now - obs.lastEventAt <= POLL_SILENCE_MS) continue;
    if (now - obs.lastPollAt < POLL_INTERVAL_MS) continue;
    obs.lastPollAt = now;
    await reconcileOnce(obs);
  }
}

/** 拉一次详情对账：阶段 status 补进 phaseStatuses（failed 亦可回放）；
 * 行消失（已删/未入库被清理）计数到上限则标记 removed 并算一条终态信号 */
async function reconcileOnce(obs: GenerationObservation) {
  try {
    const record = await fetchPlanRecord(obs.planId);
    obs.missCount = 0;
    // outline 事件丢失（HMR/监听迟到）而 DB 阶段行齐全：用 DB 阶段数补
    // 分母，否则 isObservationTerminal 恒 false → 终态永不触发、轮询无限
    // 持续（审查 Finding-3）
    if (obs.outlinePhaseCount === 0 && record.plan.phases.length > 0) {
      obs.outlinePhaseCount = record.plan.phases.length;
    }
    // 详情的阶段状态逐位对账（只补 'complete' / 'failed'，pending 不动：
    // 行在 generating 时骨架行恒pending，语义上仍属进行中）
    record.plan.phases.forEach((phase, index) => {
      if (phase.status === "complete" || phase.status === "failed") {
        obs.phaseStatuses[index] = phase.status;
      }
    });
    if (isObservationTerminal(obs)) notifyTerminal(obs);
  } catch {
    // 拉取失败：业务错误（不存在的 id 已删/清理）与网络层错误无法区分，
    // 静默期多轮取不到才定性（COMPARE 点：rowCount 错误单轮抖动不算数）
    obs.missCount += 1;
    if (obs.missCount >= MISS_LIMIT) {
      obs.removed = true;
      obs.terminal = true;
      terminalCounter.value += 1;
    }
  }
}

/**
 * 阶段完成进度只数：complete 状态的阶段索引个数（进度条分母用
 * outlinePhaseCount；failed 位也算「已收位」时不计入 complete 数）。
 */
export function completedPhaseCount(obs: GenerationObservation): number {
  return Object.values(obs.phaseStatuses).filter((s) => s === "complete")
    .length;
}

/** 历史列表行是否「生成中」：观测存在且未终态（badge spinner） */
export function isPlanGenerating(planId: string): boolean {
  const obs = observations.value[planId];
  return !!obs && !obs.terminal && !obs.removed && obs.outlinePhaseCount >= 0;
}
