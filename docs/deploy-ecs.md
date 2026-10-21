# 云端 PostgreSQL 部署指南（阿里云 ECS）

> 学习计划云端持久化的服务端部署。约 10 分钟；完成后在 Praxis
> 「设置 → 云端数据库」填入连接信息即可使用。

## 1. 前置

- 一台阿里云 ECS（2C2G 及以上，Linux；已在安全组放行 SSH 22）
- 本机可 SSH 到 ECS

## 2. 安装 Docker（已有可跳过）

```bash
curl -fsSL https://get.docker.com | sh
sudo systemctl enable --now docker
```

## 3. 启动 PostgreSQL（含 pgvector 预留）

用 pgvector 官方镜像（= 官方 PG 17 + 向量扩展，为将来语义搜索预留）：

```bash
sudo docker run -d --name praxis-pg --restart always \
  -e POSTGRES_PASSWORD='<超级用户随机强密码，不要外用>' \
  -p 5432:5432 pgvector/pgvector:pg17
```

> 注意：这里**不**用 `POSTGRES_USER`/`POSTGRES_DB` 环境变量建应用账号——
> 那样创建的是超级用户。应用账号用下面第 4 步手工创建并最小授权。

## 4. 创建应用账号（最小权限）

```bash
sudo docker exec -it praxis-pg psql -U postgres
```

```sql
-- 应用账号：仅 praxis 库的拥有者（可建表，符合「首连自动建表」设计）
CREATE USER praxis WITH PASSWORD '<应用强密码>';
CREATE DATABASE praxis OWNER praxis;
\q
```

## 5. 阿里云安全组放行

ECS 控制台 → 安全组 → 配置规则 → 入方向添加：

| 协议 | 端口 | 授权对象 | 说明 |
| --- | --- | --- | --- |
| TCP | 5432 | 0.0.0.0/0 | 家宽 IP 会变，无法用白名单；靠强密码 + 最小权限账号收口 |

> 可选加固：映射高位端口（`-p 15432:5432`）躲扫描噪音；未来可补 TLS
> （自签证书挂载 + `ssl-mode=require`）。

## 6. 备份（二选一或都做）

- **云盘快照（推荐）**：ECS 控制台 → 存储与快照 → 自动快照策略 →
  每日一档应用到该 ECS 磁盘
- **逻辑备份**：crontab 每周 `pg_dump`：

```bash
sudo docker exec praxis-pg pg_dump -U praxis praxis | gzip > ~/praxis-$(date +\%F).sql.gz
```

## 7. Praxis 客户端配置

云端数据库连接配置在本地文件 `src-tauri/config.yaml`（不入库，模板见
`src-tauri/config.example.yaml`；打包分发时与 exe 放同一目录）：

1. 填入：
   - `host`：ECS 公网 IP；`port`：5432
   - `database`：`praxis`；`user`：`praxis`
   - `password`：密文格式（`enc:v1:…`，生成方式见 `src-tauri/src/secret.rs`
     的 `manual_seal_password` 工具；手写明文亦可直连，但不会自动转密文）
2. 重启 Praxis 生效（该文件无热加载）

首次连接会自动建表（`plans` 表 + 索引），无需手工执行任何 DDL。

## 8. 验证

- 「学习计划」页应显示空列表（不再是「云端数据库未启用」引导）
- 生成一个计划 → 点「💾 保存到云端」→ 回列表应出现该记录
