<script setup lang="ts">
import {
  nextTick,
  onMounted,
  onUnmounted,
  provide,
  ref,
  shallowRef,
  watch,
} from "vue";
import {
  BookOpen,
  CalendarCheck,
  Moon,
  NotebookPen,
  Settings,
  Sun,
} from "lucide-vue-next";
import { useAgent } from "@/composables/useAgent";
import {
  disposeProgressListener,
  ensureProgressListener,
} from "@/composables/useGenerationProgress";
import { Badge } from "@/components/ui/badge";
import AppSidebar, { type MenuItem } from "@/components/AppSidebar.vue";
import CalendarView from "@/components/CalendarView.vue";
import NotesView from "@/components/NotesView.vue";
import PlanWorkbench from "@/components/PlanWorkbench.vue";
import SettingsPage from "@/components/Settings.vue";
import StartupSplash from "@/components/StartupSplash.vue";
import type { StartupStatus } from "@/types/agent";

/**
 * App：页面骨架（保持精简——只保留 header 与 main 两块）。
 * - 上：顶部栏（品牌 + Agent 在线徽标）
 * - 下：左侧菜单栏 + 右侧页面内容（学习计划 / 打卡日历 / 笔记生成 / 设置）
 * - 「新建计划 / 生成 / 结果 / 历史查看」等业务与视图态全部收敛在
 *   PlanWorkbench.vue；本文件只负责页面级路由与全局监听生命周期
 * （全局 'plan-progress' 监听随 App 生死——页面内导航不退出订阅）。
 */
const { pingAgent, startupStatus } = useAgent();

/** Agent 服务在线状态：null = 检测中 */
const agentOnline = ref<boolean | null>(null);

/**
 * 启动页状态（2026-10-22）。
 *
 * 背景：连接池惰性创建，首个 DB 命令才触发建连与全套 DDL，远端 PG 上
 * 耗时 10s+；这段时间主界面所有页面都不可用，此前只看到转圈。Rust 侧
 * 现在在启动时主动预热并经 startup_status 暴露阶段，这里阻塞渲染主界面。
 */
const startupSnapshot = ref<StartupStatus | null>(null);
const startupElapsed = ref(0);
const startupStuck = ref(false);
/** 是否已放行进入主界面（就绪且无降级，或用户点了逃生入口） */
const startupDone = ref(false);

/** 等待超过该秒数后给出「先进入应用」，避免慢网络下干等 */
const STARTUP_STUCK_SECONDS = 30;
/** 轮询间隔：启动状态是纯内存读，300ms 足够跟手且不烧 CPU */
const STARTUP_POLL_MS = 300;
/** 连续失败上限（命令未注册等极端情况）——任何情况下都不能把用户锁在启动页 */
const STARTUP_MAX_FAILURES = 20;

/** 页面唯一标识：与 MENU_ITEMS 的 id 一一对应 */
type PageId = "plan" | "calendar" | "notes" | "settings";

/** 当前激活页面：侧栏菜单切换 */
const activePage = ref<PageId>("plan");

/** PlanWorkbench 实例（日历页「打开计划详情」跨页导航的调用口） */
const workbenchRef = ref<InstanceType<typeof PlanWorkbench> | null>(null);

/** 日历抽屉点计划名：切回学习计划页并按 id 打开该计划详情 */
async function onCalendarOpenPlan(planId: string) {
  activePage.value = "plan";
  await nextTick();
  workbenchRef.value?.openPlanById(planId);
}

/** 侧栏回传的菜单 id 收窄为页面联合（AppSidebar 以宽型 string 维护 id，菜单项受控常量，收窄安全） */
function switchPage(id: string) {
  activePage.value = id as PageId;
}

/** 侧栏折叠状态（localStorage 持久化，重启应用后保持） */
const sidebarCollapsed = ref(
  localStorage.getItem("praxis.sidebarCollapsed") === "1",
);
watch(sidebarCollapsed, (v) => {
  localStorage.setItem("praxis.sidebarCollapsed", v ? "1" : "0");
});

/** 主导航菜单（新增页面时在此追加，并在内容区按 activePage 渲染对应区域） */
const MENU_ITEMS: MenuItem[] = [
  { id: "plan", label: "学习计划", icon: BookOpen },
  { id: "calendar", label: "打卡日历", icon: CalendarCheck },
  { id: "notes", label: "笔记生成", icon: NotebookPen },
  { id: "settings", label: "设置", icon: Settings },
];

/** 当前是否暗色主题（初始读 documentElement：main.ts 已在 mount 前同步初始化） */
const isDark = ref(document.documentElement.classList.contains("dark"));

/**
 * 切换全局暗色主题：toggle documentElement 的 dark class + localStorage
 * 持久化（键与 main.ts 的 initTheme 共用）；亮态显 Moon、暗态显 Sun。
 */
function toggleTheme() {
  isDark.value = !isDark.value;
  document.documentElement.classList.toggle("dark", isDark.value);
  localStorage.setItem("praxis-theme", isDark.value ? "dark" : "light");
}

/** 学习计划页滚动宿主（回顶 / 目录跳转的实际执行元素，经 inject 供下层使用） */
const planScrollHost = shallowRef<HTMLElement | null>(null);
provide("planScrollHost", planScrollHost);

/**
 * 轮询启动状态直到就绪。
 *
 * 无降级问题直接放行；有降级（库未配置/连不上、Agent 起不来）时停住，
 * 由启动页展示原因并等用户确认——静默进入只会让人以为功能坏了。
 */
async function waitForStartup() {
  const startedAt = Date.now();
  const timer = window.setInterval(() => {
    startupElapsed.value = (Date.now() - startedAt) / 1000;
    if (startupElapsed.value >= STARTUP_STUCK_SECONDS)
      startupStuck.value = true;
  }, 100);

  let failures = 0;
  try {
    for (;;) {
      try {
        const snapshot = await startupStatus();
        failures = 0;
        startupSnapshot.value = snapshot;
        if (snapshot.ready) {
          if (!snapshot.degraded) startupDone.value = true;
          break;
        }
      } catch {
        failures += 1;
        if (failures >= STARTUP_MAX_FAILURES) {
          startupDone.value = true;
          break;
        }
      }
      await new Promise((resolve) => setTimeout(resolve, STARTUP_POLL_MS));
    }
  } finally {
    window.clearInterval(timer);
  }
}

/** 刷新 Agent 在线徽标（未放行进入时不探测，避免无谓启停） */
async function refreshAgentOnline() {
  if (!startupDone.value) return;
  agentOnline.value = await pingAgent();
}

/** 启动页逃生入口：降级确认或等待超时后放行 */
function finishStartup() {
  startupDone.value = true;
  void refreshAgentOnline();
}

onMounted(async () => {
  // 全局进度监听先于任何提交注册（outline 可能先于 invoke 返回值到达）
  void ensureProgressListener();
  await waitForStartup();
  await refreshAgentOnline();
});

// App 卸载（窗口关闭/重建）才解除全局监听：页面内导航不退出订阅，
// 后台生成在离开页面后继续，回来时靠 store 恢复展示
onUnmounted(() => disposeProgressListener());
</script>

<template>
  <div class="flex h-screen flex-col overflow-hidden bg-background">
    <!-- 启动页：连云端库（10s+）与起 Agent 期间阻塞进入，
         此前这段等待只表现为各页面转圈，看起来像应用坏了 -->
    <StartupSplash
      v-if="!startupDone"
      :status="startupSnapshot"
      :elapsed="startupElapsed"
      :stuck="startupStuck"
      @enter="finishStartup"
    />
    <template v-else>
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
              <!-- 页面唯一 h1 归属详情页（PlanResult），顶栏品牌降为 p -->
              <p class="font-display text-base font-semibold leading-tight">
                Praxis · 学习路线规划 Agent
              </p>
              <p class="text-xs text-muted-foreground">
                理论 + 实操 + 作业 + 项目，可执行可验收
              </p>
            </div>
          </div>
          <div class="flex items-center gap-3">
            <!-- 暗色主题切换：样式对齐侧栏图标按钮（rounded-lg + hover 语义色） -->
            <button
              type="button"
              class="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
              :aria-label="isDark ? '切换到亮色主题' : '切换到暗色主题'"
              v-tip="isDark ? '切换到亮色主题' : '切换到暗色主题'"
              @click="toggleTheme"
            >
              <Sun v-if="isDark" class="h-4 w-4" />
              <Moon v-else class="h-4 w-4" />
            </button>
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
        </div>
      </header>

      <!-- 主体：左侧菜单栏 + 右侧页面内容（滚动收敛在内容区内部，侧栏独立） -->
      <div class="flex min-h-0 flex-1">
        <AppSidebar
          :model-value="activePage"
          @update:model-value="switchPage"
          v-model:collapsed="sidebarCollapsed"
          :items="MENU_ITEMS"
        />

        <!-- 学习计划页：滚动宿主在 main 上，内容由 PlanWorkbench 承载 -->
        <main
          v-if="activePage === 'plan'"
          ref="planScrollHost"
          class="scroll-stable min-w-0 flex-1 overflow-y-auto"
        >
          <PlanWorkbench ref="workbenchRef" />
        </main>

        <!-- 打卡日历页：视觉版式先行（mock 数据，功能暂不实现） -->
        <main
          v-else-if="activePage === 'calendar'"
          class="scroll-stable min-w-0 flex-1 overflow-y-auto"
        >
          <CalendarView @open-plan="onCalendarOpenPlan" />
        </main>

        <!-- 笔记生成页：空占位 -->
        <main
          v-else-if="activePage === 'notes'"
          class="scroll-stable min-w-0 flex-1 overflow-y-auto"
        >
          <NotesView />
        </main>

        <!-- 设置页：Agent 运行配置（保存即写盘 + 热加载） -->
        <main
          v-else-if="activePage === 'settings'"
          class="scroll-stable min-w-0 flex-1 overflow-y-auto"
        >
          <SettingsPage />
        </main>
      </div>
    </template>
  </div>
</template>
