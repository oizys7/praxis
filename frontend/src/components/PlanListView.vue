<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import {
  BookOpen,
  ChevronRight,
  Plus,
  RefreshCw,
  Search,
  Star,
  Trash2,
} from "lucide-vue-next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  isPlanGenerating,
  terminalCounter,
} from "@/composables/useGenerationProgress";
import {
  fetchPlanRecord,
  fetchPlanSummaries,
  removePlan,
  switchFavorite,
} from "@/composables/usePlanHistory";
import {
  LEVEL_OPTIONS,
  type CommandError,
  type PlanRecord,
  type PlanSummary,
} from "@/types/agent";

/**
 * PlanListView：学习计划列表（进入「学习计划」页的默认视图）。
 * - 搜索（防抖模糊过滤标题/目标）· 收藏置顶与切换 · 查看 · 删除
 * - 「新建」切换到生成表单（App.vue 负责两态切换）
 */
const emit = defineEmits<{
  /** 点击「新建计划」 */
  create: [];
  /** 点击「查看」：携带完整计划记录 */
  open: [record: PlanRecord];
}>();

const search = ref("");
const list = ref<PlanSummary[]>([]);
const loading = ref(false);
const error = ref<CommandError | null>(null);
/** 「查看」详情的加载中 id（防重复点击） */
const openingId = ref<string | null>(null);

onMounted(() => void refresh());

// 后台生成终态信号（任一观测计划判定完成/消失）：正在展示的相关行需要
// 刷新（title/overview/status 变化），重新拉列表保证所见即终态
watch(terminalCounter, () => {
  void refresh();
});

/** 搜索防抖 300ms */
let debounceTimer: number | undefined;
watch(search, () => {
  window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(() => void refresh(), 300);
});

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    list.value = await fetchPlanSummaries(search.value);
  } catch (e) {
    error.value = e as CommandError; // usePlanHistory 已归一
  } finally {
    loading.value = false;
  }
}

async function onOpen(item: PlanSummary) {
  if (openingId.value) return;
  openingId.value = item.id;
  try {
    emit("open", await fetchPlanRecord(item.id));
  } catch (e) {
    error.value = e as CommandError;
  } finally {
    openingId.value = null;
  }
}

async function onToggleFavorite(item: PlanSummary) {
  try {
    item.favorite = await switchFavorite(item.id);
    // 收藏置顶：本地重排，省一次请求
    list.value = [...list.value].sort(
      (a, b) => Number(b.favorite) - Number(a.favorite),
    );
  } catch (e) {
    error.value = e as CommandError;
  }
}

async function onDelete(item: PlanSummary) {
  if (!window.confirm(`删除计划「${item.title}」？此操作不可恢复。`)) return;
  try {
    await removePlan(item.id);
    list.value = list.value.filter((p) => p.id !== item.id);
  } catch (e) {
    error.value = e as CommandError;
  }
}

function levelLabel(value: string): string {
  return LEVEL_OPTIONS.find((o) => o.value === value)?.label ?? value;
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleString("zh-CN", { hour12: false });
}
</script>

<template>
  <div class="mx-auto w-full max-w-6xl space-y-5 px-6 py-8">
    <!-- 工具栏：标题 + 搜索 + 刷新 + 新建 -->
    <div class="flex flex-wrap items-center gap-3">
      <h2 class="mr-auto text-lg font-bold">学习计划</h2>
      <div class="relative">
        <Search
          class="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          v-model="search"
          type="search"
          name="plan-search"
          aria-label="搜索计划"
          autocomplete="off"
          placeholder="搜索标题 / 目标…"
          class="w-64 pl-9"
        />
      </div>
      <Button variant="outline" size="sm" :disabled="loading" @click="refresh">
        <RefreshCw class="mr-1 h-4 w-4" :class="{ 'animate-spin': loading }" />
        刷新
      </Button>
      <Button size="sm" @click="emit('create')">
        <Plus class="mr-1 h-4 w-4" />
        新建计划
      </Button>
    </div>

    <!-- 错误 -->
    <Card v-if="error" class="border-destructive/40">
      <CardContent class="space-y-2 py-6 text-center">
        <p class="font-semibold text-destructive">
          加载失败（{{ error.code }}）
        </p>
        <p class="text-sm text-muted-foreground">{{ error.message }}</p>
        <p v-if="error.hint" class="text-sm">💡 {{ error.hint }}</p>
        <Button variant="outline" size="sm" @click="refresh">重试</Button>
      </CardContent>
    </Card>

    <!-- 加载中 -->
    <div v-else-if="loading && !list.length" class="flex justify-center py-16">
      <div
        class="h-10 w-10 animate-spin rounded-full border-2 border-primary border-t-transparent"
      />
    </div>

    <!-- 空态 -->
    <Card v-else-if="!list.length" class="border-dashed">
      <CardContent class="flex flex-col items-center gap-2 py-16 text-center">
        <BookOpen class="h-9 w-9 text-primary/50" aria-hidden="true" />
        <p class="font-medium">
          {{ search ? "没有匹配的计划" : "还没有保存的计划" }}
        </p>
        <p class="max-w-sm text-sm text-muted-foreground">
          {{
            search
              ? "换个关键词试试，或清空搜索查看全部。"
              : "点击「新建计划」生成学习路线，生成后可一键保存到这里。"
          }}
        </p>
      </CardContent>
    </Card>

    <!-- 计划列表：紧凑索引行（非卡网格）。整行可点进详情，删除/收藏
         为行内次级操作（click.stop 防冒泡）；操作钮不靠 hover 隐藏
         （a11y），仅淡显，键盘 focus-within 同样全显 -->
    <div v-else class="overflow-hidden rounded-xl border bg-card">
      <div
        v-for="(item, ri) in list"
        :key="item.id"
        class="group flex cursor-pointer items-center gap-3 px-4 py-3.5 transition-colors hover:bg-secondary/50"
        :class="ri > 0 ? 'border-t' : ''"
        role="button"
        tabindex="0"
        :aria-label="`查看计划：${item.title}`"
        @click="onOpen(item)"
        @keydown.enter="onOpen(item)"
        @keydown.space.prevent="onOpen(item)"
      >
        <!-- 收藏星：激活态书页ecessary色（.stub保留 amber 语义） -->
        <button
          class="shrink-0 text-muted-foreground/50 transition-colors hover:text-amber-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :class="item.favorite ? 'text-amber-400 hover:text-amber-500' : ''"
          :title="item.favorite ? '取消收藏' : '收藏'"
          :aria-label="
            item.favorite ? `取消收藏：${item.title}` : `收藏：${item.title}`
          "
          :aria-pressed="item.favorite"
          @click.stop="onToggleFavorite(item)"
        >
          <Star class="h-4 w-4" :class="{ 'fill-amber-400': item.favorite }" />
        </button>
        <!-- 主列：标题 + 目标 -->
        <div class="min-w-0 flex-1">
          <h3
            class="font-display truncate text-[15px] font-semibold leading-tight"
          >
            {{ item.title }}
          </h3>
          <p class="mt-0.5 truncate text-xs text-muted-foreground">
            {{ item.goal }}
          </p>
        </div>
        <!-- 元信息列（窄屏隐藏避免挤断行） -->
        <div class="hidden shrink-0 items-center gap-2 sm:flex">
          <!-- 异步状态徽标套常驻 live region（display:contents 不动布局），
               出现/撤除时读屏公告。判据 wire 权威 + store 实时（2026-10-
               16 修复「实际已失败仍显示生成中」）：wire 已出终态时 store
               挂账不再嵭宾夺主，仅 wire 仍 generating 且本会话观测同真
               时才实时显示 -->
          <div aria-live="polite" class="contents">
            <Badge
              v-if="item.status === 'generating' && isPlanGenerating(item.id)"
              variant="secondary"
            >
              <span
                class="mr-1.5 h-2.5 w-2.5 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent"
              />
              生成中
            </Badge>
            <Badge v-if="item.status === 'failed'" variant="destructive">
              生成失败
            </Badge>
          </div>
          <!-- 列对齐：数字位数不同（8/12 周）会使左右行元信息左缘漂移，
               固定最小宽 + 居中 + 等宽数字保证同一列恒定 -->
          <Badge
            v-if="item.level"
            variant="secondary"
            class="min-w-20 justify-center"
          >
            {{ levelLabel(item.level) }}
          </Badge>
          <Badge
            v-if="item.weeks"
            variant="outline"
            class="min-w-16 justify-center tabular-nums"
            >{{ item.weeks }} 周</Badge
          >
          <span class="text-xs text-muted-foreground tabular-nums">{{
            formatTime(item.createdAt)
          }}</span>
        </div>
        <!-- 行尾操作：淡显 + focus-within 全显；删险用窗.confirm 兼容行内 -->
        <div
          class="flex shrink-0 items-center gap-0.5 opacity-40 transition-opacity focus-within:opacity-100 group-hover:opacity-100"
        >
          <Button
            variant="ghost"
            size="sm"
            class="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
            :title="`删除：${item.title}`"
            :aria-label="`删除：${item.title}`"
            @click.stop="onDelete(item)"
          >
            <Trash2 class="h-4 w-4" />
          </Button>
          <span
            v-if="openingId === item.id"
            class="flex h-8 w-8 items-center justify-center"
            aria-live="polite"
          >
            <span
              class="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent"
            />
          </span>
          <ChevronRight
            v-else
            class="h-5 w-5 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-primary"
            aria-hidden="true"
          />
        </div>
      </div>
    </div>
  </div>
</template>
