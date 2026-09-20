# Pair-wiseGSB 离线录波复盘

Pair-wiseGSB 是面向多台保护装置脱敏录波摘要的**离线工程复盘工具**。它帮助工程师把存在固定偏差、缓慢漂移和校时跳变的装置本地时间对齐到统一复盘时线，并在证据不足时保留“不可比较”状态。

本工具不连接真实变电站控制网络，不生成、不下发任何控制命令。

## 安装

```bash
cargo fetch --locked
```

## 演示

```bash
cargo test --locked && cargo run --locked -- --listen 127.0.0.1:5342
```

然后打开：

```text
http://127.0.0.1:5342
```

首次启动会在 `pair-wise-gsb.sqlite` 写入固定脱敏 fixture，包括普通漂移、校时跳变、不确定区间重叠、已发布案例和未决竞争候选。也可以用 `--db <path>` 指定其他 SQLite 文件。

## 数据模型

- **录波记录**：以 `(deviceId, contentFingerprint)` 唯一去重，重复导入会返回既有 `recordId` 且不新增事件。
- **候选归属**：同一条保护记录可以同时出现在多个未决扰动案例中。
- **已发布归属**：一条保护记录不能同时属于两个已发布扰动；冲突在发布事务内拒绝。
- **同步锚点**：可选择数字量边沿、频率特征或其他事件；锚点可标记为不可信，不可信锚点不参与拟合。
- **对时版本**：修改锚点或跳变只生成新版本；旧分段、旧时间线、旧评审结论保持可重放。
- **评审**：评审绑定具体版本，不因为后续锚点修改而被覆盖。

## 时钟函数

每台设备使用分段线性函数：

```text
corrected(t) = intercept_k + slope_k * t
             t ∈ [start_k, end_k)
```

- 第一段 `start = -∞`，最后一段 `end = +∞`，所有本地时间只属于一个分段。
- 相邻分段在跳变点满足 `left.end == right.start`。
- 跳变处允许不连续，但必须满足 `right(left.end) >= left(left.end)`，校正时间不能倒退。
- 恰好位于分段端点的事件按半开区间 `[start,end)` 归入右侧分段。
- 分段由可信锚点的精确共线点确定；当前实现要求同一分段的锚点共线，宁可拒绝建模，也不隐式最小二乘平滑。
- 斜率非负，因此分段内部也不会倒退。

## 区间与顺序口径

每个事件输出闭区间：

```text
[corrected(t) - clock_uncertainty - event_uncertainty,
 corrected(t) + clock_uncertainty + event_uncertainty]
```

两个区间 `A=[a0,a1]`、`B=[b0,b1]` 的关系为：

- `before`：`a1 < b0`
- `after`：`b1 < a0`
- `equal`：两个点区间完全相等
- `incomparable`：区间重叠或端点接触

系统不会使用装置 ID、导入顺序或记录 ID 打破不确定。页面展示的是候选因果偏序，而不是强行构造的全序。

## 事务语义

发布一个对时版本时，以下操作在同一个 SQLite 立即事务中完成：

1. 读取版本快照内的记录。
2. 检查这些记录是否已属于其他已发布案例。
3. 写入或更新 `published_membership`。
4. 标记版本发布时间。

冲突、崩溃或连接中断都不会暴露“版本已发布但归属只写了一半”的时线。并发发布由 SQLite 写事务和记录唯一归属约束保证只有一个案例成功。

## 主要 HTTP API

- `GET /api/state`：读取记录、事件、案例、锚点、版本、评审和偏序。
- `POST /api/records/import`：导入录波批次并按设备与内容指纹去重。
- `POST /api/cases`：创建扰动案例和初始候选。
- `POST /api/cases/{caseId}/candidates`：追加未决候选记录。
- `POST /api/anchors`：保存或替换锚点定义与事件引用。
- `POST /api/cases/{caseId}/versions`：根据当前可信锚点生成新的对时版本。
- `POST /api/versions/{versionId}/reviews`：向指定版本追加不可变评审。
- `POST /api/versions/{versionId}/publish`：在事务内发布版本和归属。

## 测试覆盖

```bash
cargo test --locked
```

测试覆盖：

- 分段端点只归一段。
- 校时跳变不允许校正时间倒退。
- 重叠不确定区间返回 `incomparable`。
- 设备 + 内容指纹幂等导入。
- 新锚点生成新版本，旧版本和评审仍可重放。
- 同一条记录发布到第二个扰动案例时被拒绝。
- 两个连接并发发布同一记录时只有一个案例成功。
