# 后台生成与可靠进度 —— 实施计划 v7（Python 任务化 + WS 主动推送版）

> 2026-10-14 晚。取代 v6：用户裁定 **Python→Rust 也改为主动通知**，不依赖任何长超时。
> 基线 dev/v0.0.1 @ b1b4684。5 个真实反馈问题（进度虚标/概述丢失/空历史条目/生成超时/
> 绿点转圈防重复）对应方案保留，传输架构按任务化重构。

## 架构总览（v7 vs v6 关键差异）

```
前端 ── Tauri 全局事件(plan-progress) ── Rust ── WS /api/generations/{id}/events ── Python 后台任务
        提交：POST /api/generations → {taskId} 毫秒级返回
                  └─ Python asyncio 后台任务跑完整 LangGraph（生命周期独立）
```

- v6 的问题：Rust 后台消费长 HTTP 流 → 连接与生成共存亡，需要 1800s 大超时
- v7：**生成所有权在 Python**，连接只是「观测通道」——断线重连按游标补齐即可，
  生成绝不因连接丢失而失败；不存在生成级超时，只有连接级健康检查（ping/pong 秒级）

## 设计定稿

### 1. Python 任务管理器（agent-service/app/tasks.py 新模块）

- `POST /api/generations`（kind: plan | regen_phase | regen_chapter，承载
  RegenRequest/PlanRequest 字段）→ 同步校验 + `{taskId}` 返回；真正执行进
  `asyncio.create_task`（dict 持引用防 GC），复用现有 graph/regenerate 函数
- **事件日志**：每任务一个有序 deque（seq 从 1 自增），事件 JSON 形状 = 现有
  StreamEvent 输出完全同构（outline/phase/chapter/done；重生成任务含 chapter 事件）；
  generate 与 regen 共享同一事件管道实现
- WS 端点 `WS /api/generations/{taskId}/events`：
  连接 → 先按客户端游标回放缓存事件（不丢不重），再直播新事件；
  服务端 30s ping/pong 保活；客户端可先 `GET /api/generations/{id}/snapshot`
  获取当前状态摘要（kind/status/buffered 起始 seq）
- 任务生命周期：finished（done/error）后日志保留 10 分钟供补拉，随后驱逐；
  同类任务并发上限沿用现有 Semaphore(3)；新增 `DELETE /api/generations/{id}` 取消
  （真实流中断 = cancelled，事件日志记 cancel 事件）
- 既有 /api/generate-plan/stream、/api/regenerate/stream **保留不删**（兼容缓冲期），
  Rust 切换后标记 deprecated
- 测试：pytest-asyncio 下任务执行用 httpx AsyncClient 测提交/快照端点；
  WS 用 FastAPI TestClient websocket_connect 测回放/直播/断线补齐
（uvicorn[standard] 的 websockets 依赖核对：缺则补）

### 2. Rust 侧（客户端 + 生命周期）

- 新依赖 tokio-tungstenite（WS 客户端）
- `generate_plan` 命令改造：
  a. pre-create plans 行 → `POST /api/generations` 拿 taskId → **同步返回 planId**
     （响应形状三端同步：`{ planId }`）
  b. spawn 后台任务：WS 连接 → 事件循环；JSON 形状与现有 StreamEvent 一致
     （serde_json 直接 from_str，parse 复用），逐事件 upsert 落库 + 
     `tauri::Emitter` 全局 emit `plan-progress`（{planId, kind, payload}）
  c. **断线重连**：指数退避（1s→2s→5s→10s，重试上限 ~8 次），重连时带已消费
     最大 seq 游标；snapshot 端点用于诊断旧任务（kind 与 planId 对号）
  d. done/error 事件 → 收尾 + emit 终态；取消事件 → 收尾 failed
  e. 收尾语义（v6 裁定保留）：**有任何 phase 行 → finalize complete**；
     零内容 → finalize failed
- **超时归位**：删除生成级 600s/1800s 规则；只留 PING_TIMEOUT 3s（提交握手）+
  WS ping/pong 无响应 → 视为断线走重连
- **重启收尾**：tauri 启动时扫描 status=generating 且 Python 侧无对应活动任务
  （启动时 Python 任务表必然为空）→ 全部 finalize failed + 前端可见
  （避免"永远转圈"的僵尸行）
- `update_outline_meta` 补 overview 参数写库（v6 发现的漏传修复）
- PLAN_TIMEOUT 常量删除（连带其单测改造）

### 3.前端（与 v6 相同，事件源换成 plan-progress）

- generate 返回 planId → 「后台生成中」形态，全局 listen 进度事件驱动骨架树
- 骨架计数改**成功阶段索引集合**（幂等，失败红点不占位）
- 章节点：complete 绿 / failed 红 / pending+generating **转圈**；重进页面从 DB 拉
  + 监听事件自动刷新；完成事件到达自动刷新视图
- regenInFlight → `Map<planId, Set<scope>>` 锁细化
- 历史条目：generating「生成中」徽标、failed「生成失败」徽标（可删除），
  不再有裸空卡片
- 概述渲染接库值
- vue-tsc/check/prettier

### 4. 不做（YAGNI）

- Python 侧任务持久化（内存态 + 重启收尾兜底足够，避免与规范化库双写冲突）
- 多消费者/多 tab 广播语义（单 webview 单监听）；WS 加密/鉴权（localhost sidecar）
- 再见原 REVIEW 第二次评审、SSE/HTTP 流新场景开发

## 任务分解（4 个任务）

### 任务 21：Python 任务管理器 + WS 端点（tasks.py/main.py/schemas.py）
- §1 全部；pytest 覆盖（提交即返/回放/直播/断线游标补齐/取消/驱逐）
- 提交 `feat: 生成与重生成任务化——后台执行、事件日志与 WS 推送`

### 任务 22：Rust WS 客户端 + 提交即返 + 收尾/重启收尾 + overview 落库
- §2 全部（协议三端同步 { planId }）；cargo test/clippy/fmt + 集成测试
  （收终语义：部分阶段 → complete 新用例）
- 提交 `feat: 生成命令任务化——WS 消费、断线重连与重启收尾`

### 任务 23：前端（§3）
- 提交 `feat: 进度事件驱动视图——转圈/红点/锁细化与历史徽标`

### 任务 24：e2e 收口
- mock uvicorn 真进程：提交即返 + WS 补齐 + 取消 + 重启收尾真库用例
- 真库 `cargo test -- --ignored`（DATABASE_URL 待用户）
- GUI 清单（v6 a-e 全保留）；真实 GLM 质量观察并裁定深入内容二次生成

## 审查重点

1. **事件不丢不重**：回放+直播的 seq 游标闭环（Rust 重连/Python 驱逐期竞态——
   驱逐仅在 done/error + 10min 后，Rust 收终即停止订阅 → 无竞态窗口）
2. **任务引用持有**：create_task 防回收；Semaphore 嵌套不重复限流；
   LangGraph 函数在任务上下文的异常包装为 error 事件
3. **落库先于 emit**：前端收事件时 DB 必已可读（详情页立即打开不空）
4. **重启收尾**：stale generating 精确判据（且避免误收 complete 派生行）
5. **测试环境 WS**：pytest/TestClient 与既有 AsyncClient 测试隔离
6. **前后端对账**：计划页拿 { planId } 形状三端一致；deprecated 流删不删的回归
