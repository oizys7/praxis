<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { Plus, RefreshCw, Search, Star, Trash2 } from "lucide-vue-next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
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
        <div class="text-4xl">🗂️</div>
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

    <!-- 计划卡片列表 -->
    <div v-else class="grid gap-4 md:grid-cols-2">
      <Card
        v-for="item in list"
        :key="item.id"
        class="animate-fade-in-up transition-shadow hover:shadow-md"
      >
        <CardContent class="space-y-3">
          <div class="flex items-start justify-between gap-2">
            <div class="min-w-0">
              <div class="flex items-center gap-1.5">
                <Star
                  v-if="item.favorite"
                  class="h-4 w-4 shrink-0 fill-amber-400 text-amber-400"
                />
                <h3 class="truncate font-semibold">{{ item.title }}</h3>
              </div>
              <p class="mt-0.5 truncate text-xs text-muted-foreground">
                🎯 {{ item.goal }}
              </p>
            </div>
            <button
              class="shrink-0 text-muted-foreground transition-colors hover:text-amber-500"
              :class="{ 'text-amber-400': item.favorite }"
              title="收藏"
              @click="onToggleFavorite(item)"
            >
              <Star
                class="h-4 w-4"
                :class="{ 'fill-amber-400': item.favorite }"
              />
            </button>
          </div>

          <p class="line-clamp-2 min-h-10 text-sm text-muted-foreground">
            {{ item.overview || "（暂无概述）" }}
          </p>

          <div class="flex flex-wrap items-center gap-2">
            <Badge v-if="item.level" variant="secondary">
              {{ levelLabel(item.level) }}
            </Badge>
            <Badge v-if="item.weeks" variant="outline">
              {{ item.weeks }} 周
            </Badge>
            <span class="ml-auto text-xs text-muted-foreground">
              {{ formatTime(item.createdAt) }}
            </span>
          </div>

          <div class="flex justify-end gap-2 border-t pt-3">
            <Button variant="destructive" size="sm" @click="onDelete(item)">
              <Trash2 class="mr-1 h-3.5 w-3.5" />
              删除
            </Button>
            <Button
              size="sm"
              :disabled="openingId === item.id"
              @click="onOpen(item)"
            >
              {{ openingId === item.id ? "加载中…" : "查看" }}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
