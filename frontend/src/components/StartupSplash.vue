<script setup lang="ts">
import { computed } from "vue";
import { AlertTriangle } from "lucide-vue-next";
import { Button } from "@/components/ui/button";
import type { StartupStatus } from "@/types/agent";

/**
 * 启动页（2026-10-22）：把真实启动序列画成三格进度轴。
 *
 * 背景：连接池是惰性创建的，首个 DB 命令才触发建连与全套 DDL，远端 PG
 * 上耗时 10s+；这段时间主界面所有页面都不可用，此前只看到转圈。现在
 * Rust 侧在启动时主动预热并经 startup_status 暴露阶段，本组件据此渲染。
 *
 * 设计取向（自习室纸面）：不编造假进度动画——进度数字来自真实阶段边界，
 * 文案与耗时都是实测值；焦点只给「进行中」那一行，完成态用书绿实心点，
 * 未开始用沙框空点。整页无卡片无阴影，只有一条发丝线。
 */
const props = defineProps<{
  /** 启动状态快照；null 表示尚未取到（首次轮询前） */
  status: StartupStatus | null;
  /** 已等待秒数（前端本地计时，后端 elapsedMs 仅在 ready 后有意义） */
  elapsed: number;
  /** 等待超时（后端迟迟未就绪），提供逃生入口 */
  stuck: boolean;
}>();

defineEmits<{ enter: [] }>();

/** 与 Rust startup.rs 的阶段标识一一对应；done 为完成态用词 */
const STAGES = [
  { key: "config", label: "读取配置", done: "已载入" },
  { key: "database", label: "连接云端数据库", done: "已连接" },
  { key: "agent", label: "启动 Agent 服务", done: "已就绪" },
] as const;

type StageState = "done" | "active" | "waiting";

const currentIndex = computed(() =>
  STAGES.findIndex((stage) => stage.key === props.status?.phase),
);

function stageState(index: number): StageState {
  if (props.status?.ready) return "done";
  const current = currentIndex.value;
  // 尚未取到状态（phase 为空）：把第一格当作进行中，避免全灰一片
  if (current < 0) return index === 0 ? "active" : "waiting";
  if (index < current) return "done";
  if (index === current) return "active";
  return "waiting";
}

function stageText(index: number, state: StageState): string {
  if (state === "done") return STAGES[index].done;
  if (state === "waiting") return "等待";
  return "进行中";
}

const progress = computed(() => props.status?.progress ?? 0);
const degraded = computed(() => props.status?.degraded ?? null);
/** 读屏播报用：后端给的真实阶段文案 */
const liveMessage = computed(() => props.status?.message || "正在启动");
const elapsedText = computed(() => props.elapsed.toFixed(1));
</script>

<template>
  <div class="flex h-full items-center justify-center bg-background px-6">
    <div class="animate-fade-in-up flex w-full max-w-sm flex-col">
      <!-- 品牌：与顶栏同一身份，serif 承载「纸面」气质 -->
      <div class="mb-8">
        <p class="font-display text-2xl leading-tight tracking-[0.01em]">
          Praxis
        </p>
        <p class="mt-1 text-[13px] text-muted-foreground">学习路线规划</p>
      </div>

      <!-- 降级面板：就绪但部分功能不可用，需用户知情后再进入 -->
      <div v-if="status?.ready && degraded" class="flex flex-col gap-4">
        <div class="flex gap-3">
          <AlertTriangle
            class="mt-0.5 h-4 w-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div class="min-w-0">
            <p class="text-sm font-medium text-foreground">
              已启动，但部分功能不可用
            </p>
            <p
              class="mt-1 break-words text-xs leading-relaxed text-muted-foreground"
            >
              {{ degraded }}
            </p>
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          class="self-start"
          @click="$emit('enter')"
        >
          仍然进入
        </Button>
      </div>

      <!-- 进度面板 -->
      <div v-else class="flex flex-col">
        <ul class="flex flex-col gap-3.5">
          <li
            v-for="(stage, index) in STAGES"
            :key="stage.key"
            class="flex items-center gap-3"
          >
            <!-- 圆点：完成=书绿实心，进行中=墨蓝呼吸，等待=沙框空点 -->
            <span
              class="flex h-2.5 w-2.5 shrink-0 items-center justify-center"
              aria-hidden="true"
            >
              <span
                v-if="stageState(index) === 'done'"
                class="h-2 w-2 rounded-full bg-success"
              />
              <span
                v-else-if="stageState(index) === 'active'"
                class="h-2.5 w-2.5 animate-pulse rounded-full bg-primary/25 ring-2 ring-primary"
              />
              <span v-else class="h-2 w-2 rounded-full border border-border" />
            </span>
            <span
              class="text-[15px] leading-none"
              :class="
                stageState(index) === 'waiting'
                  ? 'text-muted-foreground/70'
                  : 'text-foreground'
              "
            >
              {{ stage.label }}
            </span>
            <span
              class="ml-auto text-xs leading-none"
              :class="{
                'text-success': stageState(index) === 'done',
                'text-primary': stageState(index) === 'active',
                'text-muted-foreground/60': stageState(index) === 'waiting',
              }"
            >
              {{ stageText(index, stageState(index)) }}
            </span>
          </li>
        </ul>

        <!-- 发丝线：纸面的分隔，不用卡片或阴影 -->
        <div class="mt-7 h-px w-full bg-border" />

        <div class="mt-4 flex flex-col gap-2">
          <div
            class="h-1 w-full overflow-hidden rounded-full bg-border/70"
            role="progressbar"
            :aria-valuenow="progress"
            aria-valuemin="0"
            aria-valuemax="100"
            :aria-label="liveMessage"
          >
            <div
              class="h-full rounded-full bg-primary transition-[width] duration-500 ease-out"
              :style="{ width: `${progress}%` }"
            />
          </div>
          <div
            class="flex items-baseline justify-between text-xs text-muted-foreground"
          >
            <span class="tabular-nums">已用时 {{ elapsedText }} 秒</span>
            <span class="tabular-nums">{{ progress }}%</span>
          </div>
        </div>

        <!-- 逃生入口：后端迟迟未就绪时不要让用户干等 -->
        <div v-if="stuck" class="mt-6 flex flex-col gap-2">
          <p class="text-xs leading-relaxed text-muted-foreground">
            云端数据库响应较慢，仍在连接。可以先进入应用，历史与日历稍后可用。
          </p>
          <Button
            variant="outline"
            size="sm"
            class="self-start"
            @click="$emit('enter')"
          >
            先进入应用
          </Button>
        </div>

        <!-- 读屏播报真实阶段文案（视觉上由三行状态表达） -->
        <p class="sr-only" role="status" aria-live="polite">
          {{ liveMessage }}
        </p>
      </div>
    </div>
  </div>
</template>
