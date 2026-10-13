<script setup lang="ts">
import { onMounted, ref, shallowRef, watch } from "vue";
import { BookOpen, Settings } from "lucide-vue-next";
import { useAgent } from "@/composables/useAgent";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import AppSidebar, { type MenuItem } from "@/components/AppSidebar.vue";
import ClarificationPanel from "@/components/ClarificationPanel.vue";
import PlanForm from "@/components/PlanForm.vue";
import PlanListView from "@/components/PlanListView.vue";
import PlanResult from "@/components/PlanResult.vue";
import SettingsPage from "@/components/Settings.vue";
import type { PlanRecord, PlanRequest } from "@/types/agent";

/**
 * App：页面骨架。
 * 上：顶部栏（品牌 + Agent 在线徽标）；下：左侧菜单栏 + 右侧页面内容。
 * 启动时通过 ping_agent 探测 Python 服务状态并展示徽标。
 */
const {
  loading,
  error,
  result,
  progress,
  storageIssue,
  generatePlan,
  pingAgent,
} = useAgent();

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
/** 最近一次生成请求（保存到云端时取 goal / level / weeks 元数据） */
const lastRequest = shallowRef<PlanRequest | null>(null);

onMounted(async () => {
  agentOnline.value = await pingAgent();
});

/** 点「新建计划」：切到生成视图（清空上次生成状态，展示干净表单） */
function onCreate() {
  historyMode.value = false;
  lastRequest.value = null;
  result.value = null;
  error.value = null;
  planView.value = "create";
}

/** 表单提交 → 调用 Tauri command（Rust → Python） */
function onSubmit(request: PlanRequest) {
  lastRequest.value = request;
  historyMode.value = false;
  void generatePlan(request);
}

/** 查看历史计划：注入已保存的计划并切到查看视图 */
function openHistoryPlan(record: PlanRecord) {
  historyMode.value = true;
  lastRequest.value = null;
  error.value = null;
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
              <!-- 加载中（含流式生成进度：outline 定总数，phaseDone 推进） -->
              <Card v-if="loading" class="animate-fade-in-up">
                <CardContent
                  class="flex flex-col items-center gap-4 py-14 text-center"
                >
                  <div
                    class="h-10 w-10 animate-spin rounded-full border-2 border-primary border-t-transparent"
                  />
                  <div>
                    <p class="font-medium">Agent 正在编排你的学习计划…</p>
                    <p class="mt-1 text-sm text-muted-foreground">
                      流程：澄清检查 → 规划生成 → 质量评审（约需 30~90 秒）
                    </p>
                  </div>
                  <!-- 生成进度：第 N/M 阶段完成 + 进度条 -->
                  <div v-if="progress" class="w-full max-w-xs">
                    <div
                      class="mb-1.5 flex justify-between text-xs text-muted-foreground"
                    >
                      <span
                        >第 {{ progress.done }}/{{
                          progress.total
                        }}
                        阶段完成</span
                      >
                      <span
                        >{{
                          Math.round(
                            (progress.done / Math.max(progress.total, 1)) * 100,
                          )
                        }}%</span
                      >
                    </div>
                    <div class="h-1.5 overflow-hidden rounded-full bg-muted">
                      <div
                        class="h-full rounded-full bg-primary transition-all duration-500"
                        :style="{
                          width: `${(progress.done / Math.max(progress.total, 1)) * 100}%`,
                        }"
                      />
                    </div>
                  </div>
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
                  <Button variant="outline" size="sm" @click="dismissError"
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
                :goal="lastRequest?.goal"
                :level="lastRequest?.level"
                :weeks="lastRequest?.weeks"
                :history-mode="historyMode"
                :storage-issue="storageIssue"
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
