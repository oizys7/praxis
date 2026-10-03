<script setup lang="ts">
import { computed, onMounted, reactive } from "vue";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { useSettings } from "@/composables/useSettings";
import {
  PROVIDER_OPTIONS,
  defaultAgentSettings,
  type AgentSettings,
  type LlmProvider,
} from "@/types/agent";

/**
 * Settings：Agent 运行配置页。
 * 保存即写盘（settings.json）+ 热加载（POST /api/config），无需重启应用。
 * 五家提供商各自的 Key / 模型名独立保存，切换提供商互不覆盖。
 */
const { settings, loading, loadError, saving, saveNotice, loadSettings, saveSettings } =
  useSettings();

/** 本地草稿：全量字段常驻内存，仅按当前提供商渲染对应输入框 */
const draft = reactive<AgentSettings>(defaultAgentSettings());

onMounted(async () => {
  await loadSettings();
  if (settings.value) Object.assign(draft, settings.value);
});

/** 各提供商在 AgentSettings 中对应的字段名（字面量联合，保证类型安全索引） */
const PROVIDER_FIELDS = {
  openai: { key: "openaiApiKey", model: "openaiModel" },
  anthropic: { key: "anthropicApiKey", model: "anthropicModel" },
  deepseek: { key: "deepseekApiKey", model: "deepseekModel" },
  glm: { key: "glmApiKey", model: "glmModel" },
  opencode: { key: "opencodeApiKey", model: "opencodeModel" },
} as const;

/** 当前提供商元数据（下拉展示与说明文案） */
const providerMeta = computed(
  () =>
    PROVIDER_OPTIONS.find((p) => p.id === draft.llmProvider) ??
    PROVIDER_OPTIONS[0]!,
);

/** 当前提供商的字段名映射 */
const currentFields = computed(() => PROVIDER_FIELDS[draft.llmProvider]);

/** 当前提供商 API Key（computed 代理：切提供商自动带出该家已存 Key） */
const currentApiKey = computed({
  get: () => draft[currentFields.value.key],
  set: (v: string) => {
    draft[currentFields.value.key] = v;
  },
});

/** 当前提供商模型名 */
const currentModel = computed({
  get: () => draft[currentFields.value.model],
  set: (v: string) => {
    draft[currentFields.value.model] = v;
  },
});

function onProviderChange(provider: LlmProvider) {
  draft.llmProvider = provider;
}

function onSubmit() {
  void saveSettings({ ...draft });
}
</script>

<template>
  <div class="mx-auto w-full max-w-6xl px-6 py-8">
    <Card class="animate-fade-in-up">
      <CardHeader>
        <CardTitle>设置</CardTitle>
        <CardDescription>
          Agent
          运行配置。保存后立即热加载到服务，无需重启应用；Key 仅保存在本机
          settings.json。
        </CardDescription>
      </CardHeader>

      <CardContent>
        <!-- 加载中 -->
        <p v-if="loading" class="py-8 text-center text-sm text-muted-foreground">
          正在读取设置…
        </p>

        <!-- 读取失败 -->
        <p v-else-if="loadError" class="py-8 text-center text-sm text-destructive">
          读取设置失败：{{ loadError }}
        </p>

        <form v-else class="max-w-xl space-y-5" @submit.prevent="onSubmit">
          <!-- 提供商 -->
          <div class="space-y-1.5">
            <Label for="provider">LLM 提供商</Label>
            <Select
              id="provider"
              :model-value="draft.llmProvider"
              :options="PROVIDER_OPTIONS.map((p) => ({ value: p.id, label: p.label }))"
              @update:model-value="
                (v: string) => onProviderChange(v as LlmProvider)
              "
            />
            <p class="text-xs text-muted-foreground">
              {{ providerMeta.endpointNote }}
            </p>
          </div>

          <!-- API Key -->
          <div class="space-y-1.5">
            <Label for="apiKey">{{ providerMeta.label }} API Key</Label>
            <Input
              id="apiKey"
              v-model="currentApiKey"
              type="password"
              autocomplete="off"
              :placeholder="providerMeta.keyPlaceholder"
            />
          </div>

          <!-- 模型名 -->
          <div class="space-y-1.5">
            <Label for="model">模型名</Label>
            <Input
              id="model"
              v-model="currentModel"
              :placeholder="providerMeta.modelPlaceholder"
            />
            <p class="text-xs text-muted-foreground">
              每家提供商的模型名独立保存，切换提供商时自动带出该家的配置。
            </p>
          </div>

          <!-- 离线演示模式 -->
          <div class="flex items-center gap-2">
            <input
              id="mock"
              v-model="draft.allowMockLlm"
              type="checkbox"
              class="h-4 w-4 accent-[hsl(var(--primary))]"
            />
            <Label for="mock">离线演示模式（不调用 LLM，返回确定性 mock 计划）</Label>
          </div>

          <!-- 保存 -->
          <div class="flex items-center gap-3 pt-2">
            <Button type="submit" :disabled="saving">
              <span
                v-if="saving"
                class="mr-2 h-3.5 w-3.5 animate-spin rounded-full border-2 border-primary-foreground border-t-transparent"
              />
              保存并热加载
            </Button>

            <Badge v-if="saveNotice?.kind === 'ok'" variant="success">
              ✓ {{ saveNotice.text }}
            </Badge>
            <Badge v-else-if="saveNotice?.kind === 'warn'" variant="destructive">
              {{ saveNotice.text }}
            </Badge>
          </div>
        </form>
      </CardContent>
    </Card>
  </div>
</template>
