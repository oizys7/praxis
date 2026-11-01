# Praxis 代码重构：职责分层与胶水消除

## 原则（用户指定）

1. 异常处理与业务逻辑解耦
2. 胶水/补丁代码用设计从源头消除
3. 可读性、可维护性优先
4. 消除重复
5. **Python 仅做任务编排与 AI 调用，其余归 Rust**

## 现状问题

### Python（编排层 —— 保留但减脂）

| 问题 | 位置 | 手法 |
|---|---|---|
| 结构化调用 5 步样板 ×3 | nodes.py 三节点 | 抽 `invoke_structured()` 统一封装（prompt→render→messages→ainvoke→解析重试） |
| 异常翻译耦合在 HTTP 端点 | main.py generate_plan 捕 httpx 异常 | 下沉到 llm.py 调用层（顺带修复 openai.APITimeoutError 漏网成 500 的真 bug） |
| `AgentServiceError` 定义错位 | nodes.py（被 main.py 反向依赖） | 独立 `app/errors.py` |
| camelCase 魔法推导（出过 bug） | config.py `update_runtime` | 显式映射表 `_RUNTIME_FIELDS`（协议本就三端显式对齐） |
| 五分支构造重复参数 ×5 | llm.py | 工厂表 + OpenAI 兼容构造辅助 |

### Rust（非编排逻辑 —— 从 commands.rs 拆出）

| 问题 | 位置 | 手法 |
|---|---|---|
| 466 行混杂协议/错误/sidecar/HTTP/设置 | commands.rs | 拆 `protocol.rs`、`error.rs`、`sidecar.rs`、`agent_client.rs` |
| HTTP 调用样板 ×3 | generate_plan / ping_agent / save_settings | agent_client 统一封装（url/req_id/超时/日志） |

## 批次

1. **Python AI 调用层**：errors.py → llm.py（工厂 + invoke_structured + 异常翻译下沉）→ nodes.py 三节点减脂 → main.py 减脂 → config.py 显式映射
2. **Rust 拆分**：protocol.rs / error.rs / sidecar.rs / agent_client.rs，commands.rs 只留命令壳
3. **验证**：npm run check 全绿 + mock 模式端到端冒烟 + 协议三端对齐复查

前端与提示词 YAML 不动。
