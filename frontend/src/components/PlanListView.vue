<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import {
  Check,
  ChevronDown,
  ChevronRight,
  FolderInput,
  FolderPlus,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Star,
  Target,
  Trash2,
  X,
} from "lucide-vue-next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import LoadingState from "@/components/LoadingState.vue";
import {
  isPlanGenerating,
  terminalCounter,
} from "@/composables/useGenerationProgress";
import {
  createPlanGroup,
  fetchPlanGroups,
  fetchPlanRecord,
  fetchPlanSummaries,
  removePlan,
  removePlanGroup,
  renamePlanGroup,
  setPlanGroup,
  switchFavorite,
} from "@/composables/usePlanHistory";
import {
  LEVEL_OPTIONS,
  type CommandError,
  type PlanGroup,
  type PlanRecord,
  type PlanSummary,
} from "@/types/agent";

/**
 * PlanListView：学习计划（进入本页的默认视图）。
 * - 搜索（防抖模糊过滤标题/目标）· 收藏置顶与切换 · 查看 · 删除
 * - 「新建」切换到生成表单（PlanWorkbench 负责两态切换）
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

/**
 * 分组（用户自定义）：分组只是「组织视图的方式」，删组不删计划——后端外键
 * ON DELETE SET NULL 让组内计划回到未分组。空组必须能保留，所以分组是独立
 * 实体，而不是计划上的一个标签。
 */
const groups = ref<PlanGroup[]>([]);
/**
 * 折叠的分组（本地 UI 状态，不落库）。
 * 用普通对象而不是 Set：键的增删都由 reactive 的 Proxy 拦截，v-if 判断能稳定
 * 触发重渲染；Set 的 has() 在这里追踪不可靠（表现为点了没反应）。
 */
const collapsedGroups = ref<Record<string, boolean>>({});

/** 分组是否折叠 */
function isCollapsed(key: string): boolean {
  return collapsedGroups.value[key] === true;
}
/** 新建分组的内联输入 */
const creatingGroup = ref(false);
const newGroupName = ref("");
/** 正在重命名的分组 id 与草稿名 */
const renamingId = ref<string | null>(null);
const renameName = ref("");
/** 正在选择分组的计划 id（行内展开选择条） */
const movingId = ref<string | null>(null);

interface PlanSection {
  key: string;
  name: string;
  /** null = 未分组（不可重命名/删除） */
  groupId: string | null;
  plans: PlanSummary[];
}

/** 分区视图：有分组时按分组分区（空组也保留，方便往里放计划），未分组垫底 */
const sections = computed<PlanSection[]>(() => {
  const byGroup = new Map<string | null, PlanSummary[]>();
  for (const plan of list.value) {
    const key = plan.groupId ?? null;
    const bucket = byGroup.get(key);
    if (bucket) bucket.push(plan);
    else byGroup.set(key, [plan]);
  }
  const out: PlanSection[] = groups.value.map((g) => ({
    key: g.id,
    name: g.name,
    groupId: g.id,
    plans: byGroup.get(g.id) ?? [],
  }));
  const ungrouped = byGroup.get(null) ?? [];
  if (ungrouped.length) {
    out.push({
      key: "__ungrouped__",
      name: "未分组",
      groupId: null,
      plans: ungrouped,
    });
  }
  return out;
});

/** 没有任何自定义分组时不显示分区标题（保持原列表观感） */
const showGroupHeaders = computed(() => groups.value.length > 0);

async function refreshGroups() {
  try {
    groups.value = await fetchPlanGroups();
  } catch (e) {
    error.value = e as CommandError;
  }
}

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    const [items, gs] = await Promise.all([
      fetchPlanSummaries(search.value),
      fetchPlanGroups(),
    ]);
    list.value = items;
    groups.value = gs;
  } catch (e) {
    error.value = e as CommandError; // usePlanHistory 已归一
  } finally {
    loading.value = false;
  }
}

async function onCreateGroup() {
  const name = newGroupName.value.trim();
  if (!name) {
    creatingGroup.value = false;
    return;
  }
  try {
    await createPlanGroup(name);
    newGroupName.value = "";
    creatingGroup.value = false;
    await refreshGroups();
  } catch (e) {
    error.value = e as CommandError;
  }
}

function startRename(section: PlanSection) {
  if (!section.groupId) return;
  renamingId.value = section.groupId;
  renameName.value = section.name;
}

/**
 * Enter 提交重命名。表单原生隐式提交依赖浏览器默认行为（部分自动化
 * 键盘事件不会触发），显式处理更可靠；IME 组合中的 Enter（选词确认）
 * 不提交。
 */
function onRenameEnter(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) return;
  event.preventDefault();
  void commitRename();
}

async function commitRename() {
  const id = renamingId.value;
  if (!id) return;
  const name = renameName.value.trim();
  renamingId.value = null;
  if (!name) return;
  try {
    await renamePlanGroup(id, name);
    await refreshGroups();
  } catch (e) {
    error.value = e as CommandError;
  }
}

async function onDeleteGroup(section: PlanSection) {
  if (!section.groupId) return;
  const tip = section.plans.length
    ? `删除分组「${section.name}」？组内 ${section.plans.length} 份计划会回到「未分组」，不会被删除。`
    : `删除分组「${section.name}」？`;
  if (!window.confirm(tip)) return;
  try {
    await removePlanGroup(section.groupId);
    await Promise.all([refreshGroups(), refresh()]);
  } catch (e) {
    error.value = e as CommandError;
  }
}

function toggleGroup(key: string) {
  const next = { ...collapsedGroups.value };
  if (next[key]) delete next[key];
  else next[key] = true;
  collapsedGroups.value = next;
}

async function moveTo(plan: PlanSummary, groupId: string | null) {
  movingId.value = null;
  if ((plan.groupId ?? null) === groupId) return;
  try {
    await setPlanGroup(plan.id, groupId);
    plan.groupId = groupId;
    await refreshGroups();
  } catch (e) {
    error.value = e as CommandError;
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
    await refreshGroups();
  } catch (e) {
    error.value = e as CommandError;
  }
}

function levelLabel(value: string): string {
  return LEVEL_OPTIONS.find((o) => o.value === value)?.label ?? value;
}

/** 中文短日期格式（行尾时间列：秒级精度对决策无用，「10 月 16 日」即可） */
const DATE_FMT = new Intl.DateTimeFormat("zh-CN", {
  month: "long",
  day: "numeric",
});

function formatTime(iso: string): string {
  return DATE_FMT.format(new Date(iso));
}
</script>

<template>
  <div class="page-shell space-y-8">
    <header class="flex flex-wrap items-center justify-between gap-5">
      <div>
        <h1 class="page-heading">学习计划</h1>
        <p class="page-description">
          收好每一个学习目标，从上次停下的地方继续。
        </p>
      </div>
      <Button class="min-h-11 gap-2 px-5" @click="emit('create')">
        <Plus class="h-4 w-4" aria-hidden="true" />
        新建计划
      </Button>
    </header>

    <div class="space-y-3">
      <div class="flex flex-wrap items-center gap-3">
        <div
          class="relative min-w-0 basis-full sm:max-w-md sm:flex-1 sm:basis-auto"
        >
          <Search
            class="pointer-events-none absolute left-3.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            v-model="search"
            type="search"
            name="plan-search"
            aria-label="搜索计划标题或目标"
            autocomplete="off"
            placeholder="搜索标题或学习目标"
            class="h-11 bg-card pl-10 shadow-none"
          />
        </div>
        <div class="flex flex-wrap items-center gap-2 sm:ml-auto">
          <Button
            variant="ghost"
            class="min-h-11 gap-2 text-muted-foreground"
            :aria-expanded="creatingGroup"
            @click="creatingGroup = !creatingGroup"
          >
            <FolderPlus class="h-4 w-4" aria-hidden="true" />
            新建分组
          </Button>
          <Button
            variant="ghost"
            class="min-h-11 gap-2 text-muted-foreground"
            :disabled="loading"
            @click="refresh"
          >
            <RefreshCw
              class="h-4 w-4 motion-reduce:animate-none"
              :class="{ 'animate-spin': loading }"
              aria-hidden="true"
            />
            刷新
          </Button>
        </div>
      </div>
      <form
        v-if="creatingGroup"
        class="flex flex-wrap items-center gap-2 rounded-xl border bg-card p-3"
        @submit.prevent="onCreateGroup"
      >
        <Input
          v-model="newGroupName"
          placeholder="分组名称"
          aria-label="新分组名称"
          class="h-11 min-w-0 flex-1 basis-40 shadow-none"
          @keydown.esc="creatingGroup = false"
        />
        <Button type="submit" variant="outline" class="min-h-11"
          >创建分组</Button
        >
        <Button
          type="button"
          variant="ghost"
          class="min-h-11"
          @click="creatingGroup = false"
          >取消</Button
        >
      </form>
      <p class="text-xs text-muted-foreground" aria-live="polite">
        {{ search ? "搜索结果" : "全部计划" }} {{ list.length }} 份<span
          v-if="!search"
          >，收藏优先显示</span
        >
      </p>
    </div>

    <!-- 错误 -->
    <Card v-if="error" class="border-destructive/40" role="alert">
      <CardContent class="space-y-2 py-6 text-center">
        <p class="font-semibold text-destructive">
          暂时无法完成操作（{{ error.code }}）
        </p>
        <p class="text-sm text-muted-foreground">{{ error.message }}</p>
        <p v-if="error.hint" class="text-sm">{{ error.hint }}</p>
        <Button variant="outline" size="sm" @click="refresh">重试</Button>
      </CardContent>
    </Card>

    <!-- 加载中（全站统一 LoadingState） -->
    <LoadingState
      v-else-if="loading && !list.length"
      text="正在加载学习计划…"
    />

    <!-- 空态 -->
    <Card
      v-else-if="!list.length"
      class="rounded-2xl border-border/70 shadow-none"
    >
      <CardContent
        class="flex flex-col items-center px-6 py-16 text-center sm:py-20"
      >
        <div
          class="mb-6 flex h-20 w-16 items-center justify-center rounded-r-xl rounded-l-md border border-primary/15 bg-primary/5 text-primary"
        >
          <Target class="h-7 w-7" aria-hidden="true" />
        </div>
        <h3 class="font-display text-xl font-semibold">
          {{ search ? "没有找到这份计划" : "给新的目标留一个位置" }}
        </h3>
        <p class="mt-3 max-w-sm text-sm leading-7 text-muted-foreground">
          {{
            search
              ? "试试其他关键词，或查看全部计划。"
              : "创建你的第一份学习计划，生成后会自动保存在这里。"
          }}
        </p>
        <Button
          v-if="search"
          variant="outline"
          class="mt-6 min-h-11"
          @click="search = ''"
          >清空搜索</Button
        >
        <Button
          v-else
          variant="outline"
          class="mt-6 min-h-11 gap-2"
          @click="emit('create')"
          ><Plus class="h-4 w-4" aria-hidden="true" />新建计划</Button
        >
      </CardContent>
    </Card>

    <!-- 页面统一滚动；空分组仍可管理，不因列表为空而隐藏。 -->
    <div
      v-if="
        !error && (!loading || list.length) && (list.length || groups.length)
      "
      class="overflow-hidden rounded-2xl border bg-card"
      :aria-busy="loading"
    >
      <section v-for="section in sections" :key="section.key">
        <div
          v-if="showGroupHeaders"
          class="flex items-center gap-2 border-b bg-secondary/40 px-4 py-2 sm:px-6"
        >
          <form
            v-if="renamingId && renamingId === section.groupId"
            class="flex min-w-0 flex-1 items-center gap-1"
            @submit.prevent="commitRename"
          >
            <Input
              v-model="renameName"
              class="h-11 min-w-0 flex-1 shadow-none"
              aria-label="分组新名称"
              @keydown.enter="onRenameEnter"
              @keydown.esc="renamingId = null"
            />
            <button
              type="submit"
              aria-label="保存分组名称"
              class="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-primary hover:bg-primary/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <Check class="h-4 w-4" aria-hidden="true" />
            </button>
            <button
              type="button"
              aria-label="取消重命名"
              class="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-secondary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              @click="renamingId = null"
            >
              <X class="h-4 w-4" aria-hidden="true" />
            </button>
          </form>
          <button
            v-else
            type="button"
            class="flex min-h-11 min-w-0 flex-1 items-center gap-2 rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :aria-expanded="!isCollapsed(section.key)"
            @click="toggleGroup(section.key)"
          >
            <ChevronDown
              class="h-4 w-4 shrink-0 text-muted-foreground"
              :class="{ '-rotate-90': isCollapsed(section.key) }"
              aria-hidden="true"
            />
            <span class="truncate text-sm font-medium">{{ section.name }}</span>
            <span class="shrink-0 text-xs tabular-nums text-muted-foreground">{{
              section.plans.length
            }}</span>
          </button>
          <template v-if="section.groupId && !renamingId">
            <Button
              variant="ghost"
              class="h-11 w-11 shrink-0 p-0 text-muted-foreground hover:text-primary"
              v-tip="`重命名：${section.name}`"
              :aria-label="`重命名分组：${section.name}`"
              @click="startRename(section)"
              ><Pencil class="h-4 w-4" aria-hidden="true"
            /></Button>
            <Button
              variant="ghost"
              class="h-11 w-11 shrink-0 p-0 text-muted-foreground hover:text-destructive"
              v-tip="`删除分组：${section.name}`"
              :aria-label="`删除分组：${section.name}`"
              @click="onDeleteGroup(section)"
              ><Trash2 class="h-4 w-4" aria-hidden="true"
            /></Button>
          </template>
        </div>
        <!-- 折叠判断放在循环外层：每段只判一次，结构也更直白 -->
        <div v-if="!isCollapsed(section.key)">
          <p
            v-if="!section.plans.length"
            class="px-6 py-8 text-sm text-muted-foreground"
          >
            {{
              search
                ? "此分组没有匹配的计划。"
                : "分组里还没有计划，可以通过计划行的「移动到分组」整理到这里。"
            }}
          </p>
          <template v-for="(item, ri) in section.plans" :key="item.id">
            <div
              class="group relative grid cursor-pointer grid-cols-[minmax(0,1fr)] items-center gap-x-4 gap-y-3 px-4 py-6 transition-colors hover:bg-secondary/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:grid-cols-[minmax(0,1fr)_auto] sm:gap-x-5 sm:px-6 sm:py-7"
              :class="ri > 0 ? 'border-t' : ''"
              role="button"
              tabindex="0"
              :aria-label="`查看计划：${item.title}`"
              @click="onOpen(item)"
              :aria-busy="openingId === item.id"
              @keydown.enter.self.prevent="onOpen(item)"
              @keydown.space.self.prevent="onOpen(item)"
            >
              <!-- 选中指示条：hover/键盘焦点时左缘墨蓝细条淡入（选中感） -->
              <span
                class="absolute inset-y-2 left-0 w-0.5 rounded-full bg-primary opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100"
                aria-hidden="true"
              />
              <!-- 主列：标题 + 目标（按学习目标整理与规划，不用书籍隐喻） -->
              <div class="min-w-0 flex-1">
                <h3
                  class="line-clamp-2 break-words text-[17px] font-semibold leading-7"
                >
                  {{ item.title }}
                </h3>
                <p
                  class="mt-1 line-clamp-2 break-words text-sm leading-6 text-muted-foreground"
                >
                  {{ item.goal }}
                </p>
                <!-- 窄屏仍保留状态和学习时长。 -->
                <div
                  class="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 text-xs text-muted-foreground"
                >
                  <!-- 异步状态徽标套常驻 live region（display:contents 不动布局），
               出现/撤除时读屏公告。判据 wire 权威 + store 实时（2026-10-
               16 修复「实际已失败仍显示生成中」）：wire 已出终态时 store
               挂账不再嵭宾夺主，仅 wire 仍 generating 且本会话观测同真
               时才实时显示 -->
                  <div aria-live="polite" class="contents">
                    <Badge
                      v-if="
                        item.status === 'generating' &&
                        isPlanGenerating(item.id)
                      "
                      variant="secondary"
                    >
                      <span
                        class="mr-1.5 h-2.5 w-2.5 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent"
                      />
                      生成中
                    </Badge>
                    <!-- 三态：generating / partial / failed。
                       partial 不是"失败的近似"——它有可用内容，只是缺了几章，
                       所以用中性色而不是警示色，措辞也直说"部分完成" -->
                    <Badge
                      v-if="item.status === 'partial'"
                      variant="outline"
                      class="text-muted-foreground"
                    >
                      部分完成
                    </Badge>
                    <Badge
                      v-if="item.status === 'failed'"
                      variant="destructive"
                    >
                      生成失败
                    </Badge>
                  </div>
                  <span v-if="item.level">{{ levelLabel(item.level) }}</span>
                  <span v-if="item.weeks" class="tabular-nums"
                    >{{ item.weeks }} 周学习</span
                  >
                  <time :datetime="item.createdAt"
                    >{{ formatTime(item.createdAt) }} 创建</time
                  >
                </div>
              </div>
              <!-- 操作常驻可见，键盘事件不触发行打开。 -->
              <div
                class="col-start-2 flex items-center gap-1 sm:col-start-auto"
              >
                <button
                  type="button"
                  class="flex h-11 w-11 items-center justify-center rounded-lg transition-colors hover:bg-secondary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  :class="
                    item.favorite ? 'text-primary' : 'text-muted-foreground'
                  "
                  v-tip="item.favorite ? '取消收藏' : '收藏'"
                  :aria-label="`${item.favorite ? '取消收藏' : '收藏'}：${item.title}`"
                  :aria-pressed="item.favorite"
                  @click.stop="onToggleFavorite(item)"
                >
                  <Star
                    class="h-4 w-4"
                    :class="{ 'fill-current': item.favorite }"
                    aria-hidden="true"
                  />
                </button>
                <Button
                  variant="ghost"
                  size="sm"
                  class="h-11 w-11 p-0 text-muted-foreground hover:text-destructive"
                  v-tip="`删除：${item.title}`"
                  :aria-label="`删除：${item.title}`"
                  @click.stop="onDelete(item)"
                >
                  <Trash2 class="h-4 w-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  class="h-11 w-11 p-0 text-muted-foreground hover:text-primary"
                  v-tip="`移动到分组：${item.title}`"
                  :aria-label="`移动到分组：${item.title}`"
                  :aria-expanded="movingId === item.id"
                  @click.stop="movingId = movingId === item.id ? null : item.id"
                >
                  <FolderInput class="h-4 w-4" />
                </Button>
                <span
                  v-if="openingId === item.id"
                  class="flex h-8 w-8 items-center justify-center"
                  role="status"
                >
                  <span class="sr-only">正在打开计划</span>
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
            <!-- 移组选择条：行内展开而不是下拉/弹窗——不受滚动容器裁剪影响，
             也不必为定位再引一个浮层 -->
            <div
              v-if="movingId === item.id"
              class="flex flex-wrap items-center gap-2 border-t bg-secondary/40 px-4 py-4 sm:px-6"
            >
              <span class="text-xs text-muted-foreground">移到：</span>
              <Button
                v-for="g in groups"
                :key="g.id"
                variant="outline"
                size="sm"
                class="min-h-11 max-w-full whitespace-normal break-words"
                :class="{
                  'border-primary text-primary': g.id === item.groupId,
                }"
                @click.stop="moveTo(item, g.id)"
              >
                {{ g.name }}
              </Button>
              <Button
                variant="outline"
                size="sm"
                class="min-h-11 max-w-full whitespace-normal break-words"
                :class="{ 'border-primary text-primary': !item.groupId }"
                @click.stop="moveTo(item, null)"
              >
                未分组
              </Button>
            </div>
          </template>
        </div>
      </section>
    </div>
  </div>
</template>
