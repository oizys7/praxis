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

/**
 * App：页面骨架（保持精简——只保留 header 与 main 两块）。
 * - 上：顶部栏（品牌 + Agent 在线徽标）
 * - 下：左侧菜单栏 + 右侧页面内容（学习计划 / 打卡日历 / 笔记生成 / 设置）
 * - 「新建计划 / 生成 / 结果 / 历史查看」等业务与视图态全部收敛在
 *   PlanWorkbench.vue；本文件只负责页面级路由与全局监听生命周期
 * （全局 'plan-progress' 监听随 App 生死——页面内导航不退出订阅）。
 */
const { pingAgent } = useAgent();

/** Agent 服务在线状态：null = 检测中 */
const agentOnline = ref<boolean | null>(null);

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

onMounted(async () => {
  // 全局进度监听先于任何提交注册（outline 可能先于 invoke 返回值到达）
  void ensureProgressListener();
  agentOnline.value = await pingAgent();
});

// App 卸载（窗口关闭/重建）才解除全局监听：页面内导航不退出订阅，
// 后台生成在离开页面后继续，回来时靠 store 恢复展示
onUnmounted(() => disposeProgressListener());
</script>

<template>
  <div class="flex h-screen flex-col overflow-hidden bg-background">
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
            :title="isDark ? '切换到亮色主题' : '切换到暗色主题'"
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
        class="min-w-0 flex-1 overflow-y-auto"
      >
        <PlanWorkbench ref="workbenchRef" />
      </main>

      <!-- 打卡日历页：视觉版式先行（mock 数据，功能暂不实现） -->
      <main
        v-else-if="activePage === 'calendar'"
        class="min-w-0 flex-1 overflow-y-auto"
      >
        <CalendarView @open-plan="onCalendarOpenPlan" />
      </main>

      <!-- 笔记生成页：空占位 -->
      <main
        v-else-if="activePage === 'notes'"
        class="min-w-0 flex-1 overflow-y-auto"
      >
        <NotesView />
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
