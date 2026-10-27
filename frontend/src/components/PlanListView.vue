<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import {
  BookOpen,
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
  <div class="mx-auto w-full max-w-6xl space-y-5 px-6 py-8 2xl:max-w-7xl">
    <!-- 页头：衬线大标题 + 摘行；操作按钮右置同一行，hairline 收边 -->
    <div class="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
      <div>
        <h2 class="font-display text-3xl font-semibold tracking-tight">
          学习计划
        </h2>
        <p class="mt-1 text-sm text-muted-foreground">
          {{ list.length }} 份计划，按最近更新排序
        </p>
      </div>
      <div class="flex items-center gap-3">
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
        <template v-if="creatingGroup">
          <Input
            v-model="newGroupName"
            placeholder="分组名称"
            aria-label="新分组名称"
            class="w-40"
            @keydown.enter="onCreateGroup"
            @keydown.esc="creatingGroup = false"
          />
          <Button size="sm" @click="onCreateGroup">确定</Button>
          <Button variant="ghost" size="sm" @click="creatingGroup = false">
            取消
          </Button>
        </template>
        <Button
          v-else
          variant="outline"
          size="sm"
          @click="creatingGroup = true"
        >
          <FolderPlus class="mr-1 h-4 w-4" />
          新建分组
        </Button>
        <Button
          variant="outline"
          size="sm"
          :disabled="loading"
          @click="refresh"
        >
          <RefreshCw
            class="mr-1 h-4 w-4"
            :class="{ 'animate-spin': loading }"
          />
          刷新
        </Button>
        <Button size="sm" @click="emit('create')">
          <Plus class="mr-1 h-4 w-4" />
          新建计划
        </Button>
      </div>
    </div>
    <div class="h-px bg-primary/15" role="presentation" />

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

    <!-- 加载中（全站统一 LoadingState） -->
    <LoadingState
      v-else-if="loading && !list.length"
      text="正在加载学习计划…"
    />

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
    <!-- 最大高度 + 内部滚动：计划一多时页面不再被拉长，页头始终留在视野里 -->
    <div
      v-else
      class="max-h-[calc(100vh_-_17rem)] overflow-x-hidden overflow-y-auto rounded-xl border bg-card"
    >
      <section v-for="section in sections" :key="section.key">
        <!-- 组标题：吸附在滚动容器顶部，滚动时始终知道当前在哪个分组 -->
        <div
          v-if="showGroupHeaders"
          class="sticky top-0 z-10 flex items-center gap-2 border-b bg-secondary px-4 py-2"
        >
          <!-- 用 div + role=button 而不是原生 button：index.css 有一条全局的
               button:active { scale(0.97) } 按压反馈（unlayered，工具类压不住），
               点组标题时整条标题栏跟着缩放，正是「太浮夸」的来源 -->
          <div
            role="button"
            tabindex="0"
            class="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :aria-expanded="!isCollapsed(section.key)"
            @click="toggleGroup(section.key)"
            @keydown.enter.prevent="toggleGroup(section.key)"
            @keydown.space.prevent="toggleGroup(section.key)"
          >
            <ChevronDown
              class="h-4 w-4 shrink-0 text-muted-foreground transition-transform"
              :class="{ '-rotate-90': isCollapsed(section.key) }"
              aria-hidden="true"
            />
            <template v-if="renamingId && renamingId === section.groupId">
              <Input
                v-model="renameName"
                class="h-7 w-40"
                aria-label="分组新名称"
                @click.stop
                @keydown.enter="commitRename"
                @keydown.esc="renamingId = null"
              />
              <Check
                class="h-4 w-4 shrink-0 cursor-pointer text-success"
                @click.stop="commitRename"
              />
              <X
                class="h-4 w-4 shrink-0 cursor-pointer text-muted-foreground"
                @click.stop="renamingId = null"
              />
            </template>
            <template v-else>
              <span class="truncate text-sm font-medium">{{
                section.name
              }}</span>
              <span class="shrink-0 text-xs tabular-nums text-muted-foreground">
                {{ section.plans.length }}
              </span>
            </template>
          </div>
          <template v-if="section.groupId && !renamingId">
            <Button
              variant="ghost"
              size="sm"
              class="h-7 w-7 p-0 text-muted-foreground hover:text-primary"
              v-tip="`重命名：${section.name}`"
              :aria-label="`重命名分组：${section.name}`"
              @click.stop="startRename(section)"
            >
              <Pencil class="h-3.5 w-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              class="h-7 w-7 p-0 text-muted-foreground hover:text-destructive"
              v-tip="`删除分组：${section.name}`"
              :aria-label="`删除分组：${section.name}`"
              @click.stop="onDeleteGroup(section)"
            >
              <Trash2 class="h-3.5 w-3.5" />
            </Button>
          </template>
        </div>
        <!-- 折叠判断放在循环外层：每段只判一次，结构也更直白 -->
        <div v-if="!isCollapsed(section.key)">
          <template v-for="(item, ri) in section.plans" :key="item.id">
            <div
              class="group relative flex cursor-pointer items-center gap-3 px-4 py-3.5 transition-colors hover:bg-secondary/50"
              :class="ri > 0 ? 'border-t' : ''"
              role="button"
              tabindex="0"
              :aria-label="`查看计划：${item.title}`"
              @click="onOpen(item)"
              @keydown.enter="onOpen(item)"
              @keydown.space.prevent="onOpen(item)"
            >
              <!-- 选中指示条：hover/键盘焦点时左缘墨蓝细条淡入（选中感） -->
              <span
                class="absolute inset-y-2 left-0 w-0.5 rounded-full bg-primary opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100"
                aria-hidden="true"
              />
              <!-- 收藏星：激活态书页ecessary色（.stub保留 amber 语义） -->
              <button
                class="shrink-0 text-muted-foreground/50 transition-colors hover:text-amber-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                :class="
                  item.favorite ? 'text-amber-400 hover:text-amber-500' : ''
                "
                v-tip="item.favorite ? '取消收藏' : '收藏'"
                :aria-label="
                  item.favorite
                    ? `取消收藏：${item.title}`
                    : `收藏：${item.title}`
                "
                :aria-pressed="item.favorite"
                @click.stop="onToggleFavorite(item)"
              >
                <Star
                  class="h-4 w-4"
                  :class="{ 'fill-amber-400': item.favorite }"
                />
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
                    v-if="
                      item.status === 'generating' && isPlanGenerating(item.id)
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
                <span
                  class="w-16 shrink-0 text-right text-xs text-muted-foreground"
                  >{{ formatTime(item.createdAt) }}</span
                >
              </div>
              <!-- 行尾操作：淡显 + focus-within 全显；删险用窗.confirm 兼容行内 -->
              <div
                class="flex shrink-0 items-center gap-0.5 opacity-40 transition-opacity focus-within:opacity-100 group-hover:opacity-100"
              >
                <Button
                  variant="ghost"
                  size="sm"
                  class="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
                  v-tip="`删除：${item.title}`"
                  :aria-label="`删除：${item.title}`"
                  @click.stop="onDelete(item)"
                >
                  <Trash2 class="h-4 w-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  class="h-8 w-8 p-0 text-muted-foreground hover:text-primary"
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
            <!-- 移组选择条：行内展开而不是下拉/弹窗——不受滚动容器裁剪影响，
             也不必为定位再引一个浮层 -->
            <div
              v-if="movingId === item.id"
              class="flex flex-wrap items-center gap-2 border-t bg-secondary/40 px-4 py-2"
            >
              <span class="text-xs text-muted-foreground">移到：</span>
              <Button
                v-for="g in groups"
                :key="g.id"
                variant="outline"
                size="sm"
                class="h-7"
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
                class="h-7"
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
