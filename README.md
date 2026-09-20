# 离线录波对时复盘

这是面向电网试验团队的离线工程复盘工具。它只读取脱敏录波摘要、维护本地 SQLite 数据库并展示复盘页面，不连接真实变电站，也不下发控制命令。

## 安装与演示

```bash
cargo fetch --locked
cargo test --locked && cargo run --locked -- --listen 127.0.0.1:5342
```

打开：

```text
http://127.0.0.1:5342
```

首次启动会在 `timeline_review.sqlite3` 中播种固定 fixture，包含三台保护装置、固定时钟偏差、缓慢漂移、一次校时跳变、确定先后关系和重叠不确定区间。也可以用 `--db <path>` 指定临时数据库。

## 复盘模型

- 原始本地时、采样率、数字量、相量特征、跳闸和重合闸事件原样保存。
- 录波可以先作为多个扰动的未决候选并存；发布对时版本后，同一条保护记录只能归属于一个已发布扰动。
- 修改锚点不会覆盖旧结论，而是创建同一扰动下递增的不可变对时版本；版本、分段、事件校正时间和归属关系在一个 SQLite 事务内提交，失败时回滚。
- 导入批次按规范化 JSON 计算 SHA-256，记录按“装置 + 记录内容指纹”去重；重复导入返回插入 0 条和跳过数量。
- 分段可以标记 `trusted: false`，表示工程师保留但不信任该锚点；该分段仍作为覆盖边界保存在版本中，但不产生校正时。每次发布至少要有一个可信分段。

## 时钟函数

每台装置按捕获顺序拥有若干分段。捕获点是 `(record_import_seq, event_seq)`，用于区分校时导致的本地时重叠或跳变。

一个可信分段由两个锚点描述：

- 本地时：`t0_local`、`t1_local`
- 校正时：`t0_corrected`、`t1_corrected`
- 不确定半径：`r0`、`r1`

对本地时 `t_local`，使用分段线性函数：

```text
t_corrected(t_local)
  = t0_corrected
  + round((t1_corrected - t0_corrected)
          * (t_local - t0_local)
          / (t1_local - t0_local))
```

半径采用端点间线性插值并向保守方向取整，事件区间为：

```text
[t_corrected - r(t_local), t_corrected + r(t_local)]
```

所有时间都是纳秒整数。段内本地时和校正时都不能倒退；跨校时跳变允许校正时不连续，但下一段起点校正时不得小于上一段终点校正时。

## 端点与区间口径

- 分段按捕获顺序覆盖事件。
- 普通分段覆盖其起点；当相邻分段共享同一个捕获端点时，该端点只能由右段计算。
- 共享端点的本地时必须相同；校正时可以不同，用于表达校时跳变。
- 不确定区间使用闭区间。
- 当且仅当 `left.max < right.min` 时判定左事件先于右事件；`right.max < left.min` 时判定后于。
- 区间重叠或端点相触都标记为“无法比较”。系统不会用装置 ID、记录顺序或校正点值并列来强行制造因果。

页面会并排显示原本地时、校正时、闭区间、装置事件、已发布/候选归属和可确定的候选因果顺序。

## HTTP API

- `GET /api/state`：读取装置、记录、事件、扰动归属、全部版本和最新版本的确定顺序。
- `POST /api/batches`：导入脱敏录波批次并按装置和内容指纹去重。
- `POST /api/disturbances`：创建扰动案例。
- `POST /api/candidates`：把记录加入未决候选。
- `POST /api/publish-alignment`：创建新的不可变对时版本，并在同一事务内发布记录归属。

## 数据存储

SQLite schema 位于 `migrations/0001_init.sql`：

- `UNIQUE(device_id, fingerprint)` 保证同一装置下内容相同的记录不重复。
- 条件唯一索引 `UNIQUE(record_id) WHERE status = 'published'` 保证已发布记录只有一个扰动归属。
- `alignment_versions`、`alignment_segments`、`event_timings` 和 `memberships` 的写入由同一事务保护，中断时不会暴露半对齐时线。

## 测试

`cargo test --locked` 覆盖：

- 固定偏差、漂移和校时跳变的分段线性映射。
- 共享端点只由右段计算、校时跳变不连续、校正时间不倒退。
- 闭区间重叠或相触时不可比较。
- 批次/记录指纹去重。
- 未决候选并存、发布后唯一归属。
- 发布失败回滚、旧版本可重放。
- 两个连接并发发布同一条记录时只有一个事务成功。
