<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, shallowRef, watch } from "vue";
import { BookOpen, Settings } from "lucide-vue-next";
import { useAgent } from "@/composables/useAgent";
import {
  completedPhaseCount,
  disposeProgressListener,
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
import AppSidebar, { type MenuItem } from "@/components/AppSidebar.vue";
import ClarificationPanel from "@/components/ClarificationPanel.vue";
import PlanForm from "@/components/PlanForm.vue";
import PlanListView from "@/components/PlanListView.vue";
import PlanResult from "@/components/PlanResult.vue";
import SettingsPage from "@/components/Settings.vue";
import { normalizeError } from "@/composables/useAgent";
import type { PlanRecord, PlanRequest, StudyPlan } from "@/types/agent";

/**
 * App：页面骨架。
 * 上：顶部栏（品牌 + Agent 在线徽标）；下：左侧菜单栏 + 右侧页面内容。
 * 启动时通过 ping_agent 探测 Python 服务状态并展示徽标；并注册全局
 * 'plan-progress' 监听（后台生成的进度流，模块级 store 观测）。
 */
const { loading, error, result, generatePlan, pingAgent } = useAgent();

/** Agent 服务在线状态：null = 检测中 */
const agentOnline = ref<boolean | null>(null);

/** 当前激活页面：侧栏菜单切换 */
const activePage = ref("plan");

/** 侧栏折叠状态（localStorage 持久化，重启应用后保持） */
const sidebarCollapsed = ref(
  localStorage.getItem("praxis.sidebarCollapsed") === "1",
);
watch(sidebarCollapsed, (v) => {
  localStorage.setItem("praxis.sidebarCollapsed", v ? "1" : "0");
});

/** 主导航菜单（新增页面时在此追加，并在内容区按 activePage 渲染对应 section） */
const MENU_ITEMS: MenuItem[] = [
  { id: "plan", label: "学习计划", icon: BookOpen },
  { id: "settings", label: "设置", icon: Settings },
];

/** 学习计划页内部视图：列表（默认） / 生成表单与结果 */
const planView = ref<"list" | "create">("list");
/** 历史查看模式：右侧展示已保存计划，隐藏左侧表单与保存按钮 */
const historyMode = ref(false);
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

onMounted(async () => {
  // 全局进度监听先于任何提交注册（outline 可能先于 invoke 返回值到达）
  void ensureProgressListener();
  agentOnline.value = await pingAgent();
});

// App 卸载（窗口关闭/重建）才解除全局监听：页面内导航不退出订阅，
// 后台生成在离开页面后继续，回来时靠 store 恢复展示
onUnmounted(() => disposeProgressListener());

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
  followPlan(submitted.planId);
}

/** 查看历史计划：注入已保存的计划并切到查看视图（携带 id 供重生成入口） */
function openHistoryPlan(record: PlanRecord) {
  historyMode.value = true;
  historyPlanId.value = record.id;
  lastRequest.value = null;
  error.value = null;
  unfollowPlan();
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
  <div
    class="flex h-screen flex-col overflow-hidden bg-gradient-to-b from-background to-muted/40"
  >
    <!-- 顶部栏 -->
    <header class="shrink-0 border-b bg-background/80 backdrop-blur">
      <div class="flex items-center justify-between px-6 py-3.5">
        <div class="flex items-center gap-3">
          <div
            class="flex h-9 w-9 items-center justify-center rounded-lg bg-primary text-base font-black text-primary-foreground"
          >
            P
          </div>
          <div>
            <h1 class="text-base font-bold leading-tight">
              Praxis · 学习路线规划 Agent
            </h1>
            <p class="text-xs text-muted-foreground">
              理论 + 实操 + 作业 + 项目，可执行可验收
            </p>
          </div>
        </div>
        <Badge
          :variant="
            agentOnline === null
              ? 'secondary'
              : agentOnline
                ? 'success'
                : 'destructive'
          "
        >
          <span class="mr-1">●</span>
          Agent 服务{{
            agentOnline === null ? "检测中…" : agentOnline ? "在线" : "离线"
          }}
        </Badge>
      </div>
    </header>

    <!-- 主体：左侧菜单栏 + 右侧页面内容（滚动收敛在内容区内部，侧栏独立） -->
    <div class="flex min-h-0 flex-1">
      <AppSidebar
        v-model="activePage"
        v-model:collapsed="sidebarCollapsed"
        :items="MENU_ITEMS"
      />

      <!-- 学习计划页：列表（默认） / 生成与查看 -->
      <main v-if="activePage === 'plan'" class="min-w-0 flex-1 overflow-y-auto">
        <PlanListView
          v-if="planView === 'list'"
          @create="onCreate"
          @open="openHistoryPlan"
        />

        <div v-else class="mx-auto w-full max-w-6xl px-6 py-8">
          <div class="mb-4">
            <Button variant="ghost" size="sm" @click="planView = 'list'">
              ← 返回列表
            </Button>
          </div>
          <div
            class="grid w-full items-start gap-6"
            :class="historyMode ? 'lg:grid-cols-1' : 'lg:grid-cols-[400px,1fr]'"
          >
            <PlanForm
              v-if="!historyMode"
              :loading="loading"
              @submit="onSubmit"
            />

            <section class="min-w-0 space-y-4">
              <!-- 后台生成进行中（提交即返后的过程页：outline 铺骨架，phaseDone 推进） -->
              <Card v-if="generatingNow" class="animate-fade-in-up">
                <CardContent
                  class="flex flex-col items-center gap-4 py-14 text-center"
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
                    ⚠ {{ genState.storageIssue }}
                  </p>
                  <!-- 生成进度：第 N/M 阶段完成 + 进度条（outline 未到则隐藏） -->
                  <div v-if="progressTotal" class="w-full max-w-xs">
                    <div
                      class="mb-1.5 flex justify-between text-xs text-muted-foreground"
                    >
                      <span
                        >第 {{ progressDone }}/{{
                          progressTotal
                        }}
                        阶段完成</span
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
                        class="h-full rounded-full bg-primary transition-all duration-500"
                        :style="{
                          width: `${(progressDone / Math.max(progressTotal, 1)) * 100}%`,
                        }"
                      />
                    </div>
                  </div>
                </CardContent>
              </Card>

              <!-- 提交在飞窗口（极短；generating 卡片就位前占位） -->
              <Card v-else-if="loading" class="animate-fade-in-up">
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
              <Card
                v-else-if="error"
                class="animate-fade-in-up border-destructive/40"
              >
                <CardContent class="space-y-3 py-8 text-center">
                  <div class="text-3xl">⚠️</div>
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
                class="animate-fade-in-up border-destructive/40"
              >
                <CardContent class="space-y-3 py-8 text-center">
                  <div class="text-3xl">⚠️</div>
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
                  <Button
                    variant="outline"
                    size="sm"
                    @click="dismissResultError"
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

              <!-- 空态 -->
              <Card v-else class="border-dashed">
                <CardContent
                  class="flex flex-col items-center gap-2 py-16 text-center"
                >
                  <div class="text-4xl">🗺️</div>
                  <p class="font-medium">你的学习路线图将在这里出现</p>
                  <p class="max-w-sm text-sm text-muted-foreground">
                    填写左侧表单并点击「生成学习计划」，Agent
                    会先检查信息是否完整，再生成、评审并输出分阶段计划。
                  </p>
                </CardContent>
              </Card>

              <!-- 大纲骨架树（渐进式渲染，骨架是过程态：终态后由 PlanResult 接管） -->
              <Card
                v-if="outlineReady"
                class="animate-fade-in-up"
                aria-live="polite"
              >
                <CardContent class="space-y-4 py-6">
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
                        <span class="text-sm font-medium">{{
                          phase.name
                        }}</span>
                        <div class="flex items-center gap-2">
                          <span
                            v-if="phase.weekRange"
                            class="text-xs text-muted-foreground"
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
                      <ul
                        v-if="phase.chapterTitles.length"
                        class="mt-2 space-y-1"
                      >
                        <li
                          v-for="(chapterTitle, ci) in phase.chapterTitles"
                          :key="ci"
                          class="flex items-center gap-2 text-sm text-muted-foreground"
                        >
                          <!-- 章节点随本阶段位：完成绿，失败红，生成中灰 -->
                          <span
                            class="h-1.5 w-1.5 shrink-0 rounded-full"
                            :class="
                              genState?.phaseStatuses[i] === 'complete'
                                ? 'bg-emerald-500'
                                : genState?.phaseStatuses[i] === 'failed'
                                  ? 'bg-destructive'
                                  : 'bg-muted-foreground/40'
                            "
                          /><span class="truncate"
                            >{{ chapterTitle
                            }}{{
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
      </main>

      <!-- 设置页：Agent 运行配置（保存即写盘 + 热加载） -->
      <main
        v-else-if="activePage === 'settings'"
        class="min-w-0 flex-1 overflow-y-auto"
      >
        <SettingsPage />
      </main>
    </div>
  </div>
</template>
