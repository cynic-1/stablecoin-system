# Claude 工作审查报告（完整）

审查时间：2026-02-25  
审查对象：`PROGRESS.md`、`prd.md`、`leap/`、`mp3bft/`、`experiments/`、`narwhal/benchmark/`  
审查方法：静态代码核对 + 实验脚本链路核对 + 本地测试复验

---

## 1. 总结结论（先给结论）

当前实现**没有严格贯彻** `prd.md` 的关键要求。主要问题不是“能不能编译”，而是：

1. 文档中多个检查点写成 PASSED，但证据与 PRD 约束不一致。  
2. LEAP 声称的核心优化（Domain-aware / Hot-Delta / 自适应背压）大多停留在模块与单测层，未真正进入主执行路径。  
3. 实验 2 的“默认运行入口”与报告主结论不一致，容易导致复现与论文叙述错位。  
4. 有 PRD 明确要求的交付物缺失（`mp3bft/NARWHAL_ANALYSIS.md`）。

换句话说：**测试能过 ≠ 需求已满足 ≠ 实验结论可信**。

---

## 2. 审查范围与复验动作

### 2.1 审查重点

1. `PROGRESS.md` 是否真实反映 `prd.md` 的刚性要求。  
2. 代码实现是否把“声称完成”的优化真正接入执行/共识主路径。  
3. 实验脚本是否与报告口径一致，数据是否有可追溯链路。  
4. 最低可运行性是否存在（测试复验）。

### 2.2 已执行复验

执行并确认：

- `cargo test -q` in `leap/` -> 23 tests passed  
- `cargo test -q` in `mp3bft/` -> 22 tests passed  
- `cargo test -q` in `e2e/` -> 4 tests passed

说明：通过上述测试仅能证明局部单测/集成测可运行，不能自动证明 PRD 红线满足。

---

## 3. 严重问题清单（按严重级别）

## HIGH-1：LEAP 三项核心优化未真正接入主执行路径

### 现象

`PROGRESS.md` 声称 Step 1.6/1.7/1.8 完成并通过验证，但主执行路径中看不到对应调用链。

### 关键证据

- 配置存在开关：  
  - `/home/ubuntu/claude_stablecoin/leap/src/config.rs:13`  
  - `/home/ubuntu/claude_stablecoin/leap/src/config.rs:14`  
  - `/home/ubuntu/claude_stablecoin/leap/src/config.rs:15`
- 执行器仅将 `enable_backpressure` 映射为固定窗口值，没有自适应控制闭环：  
  - `/home/ubuntu/claude_stablecoin/leap/src/executor.rs:242`  
  - `/home/ubuntu/claude_stablecoin/leap/src/executor.rs:247`
- `DomainPlan` 仅在其模块内定义与测试，未被执行器消费：  
  - `/home/ubuntu/claude_stablecoin/leap/src/domain_plan.rs:28`  
  - 全局搜索仅命中本文件测试与定义。
- `HotDeltaManager` 仅在模块内定义与测试，未进入执行器写路径：  
  - `/home/ubuntu/claude_stablecoin/leap/src/hot_delta.rs:12`
- `BackpressureController` 仅在模块内定义与测试，未在主路径实例化：  
  - `/home/ubuntu/claude_stablecoin/leap/src/backpressure.rs:35`
- 稳定币执行仍直接对 `Balance` 写入，不见 Delta 重写：  
  - `/home/ubuntu/claude_stablecoin/leap/src/stablecoin.rs:177`  
  - `/home/ubuntu/claude_stablecoin/leap/src/stablecoin.rs:179`

### 影响

1. `LEAP` 与 `LEAP-noDomain` / `LEAP-noHotDelta` 等 ablation 结果解释力严重不足。  
2. CP-3 与 CP-4 中“优化有效”结论可信度不足。  
3. 当前更像“带配置开关的 Block-STM 变体 + CADO 排序”，不是 PRD 描述的完整 LEAP 优化闭环。

---

## HIGH-2：CP-4 红线未满足却标记 PASSED

### PRD 硬约束

- 绝对红线：增加线程时 TPS 单调不减（至少到 16 线程）：  
  - `/home/ubuntu/claude_stablecoin/prd.md:5`
- CP-4 必须满足条件 3（单调不减）：  
  - `/home/ubuntu/claude_stablecoin/prd.md:463`
- 若 CP-4 不通过，不得进入 Phase 2：  
  - `/home/ubuntu/claude_stablecoin/prd.md:470`

### 关键证据

- `PROGRESS.md` 自己给出的 Uniform/LEAP 数据：8 线程 591K -> 16 线程 584K（下降）  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:108`
- 同一文件却写 Step 1.9 CP-4 PASSED：  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:33`  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:92`

### 影响

这是直接违反 PRD 红线的“结论与数据冲突”。该问题属于流程性与可信性双重高风险。

---

## HIGH-3：CP-4 对比对象偏离 PRD（LEAP vs LEAP-base 代替 LEAP vs Block-STM）

### PRD 要求

CP-4 明确要求 `LEAP vs Block-STM`：  
- `/home/ubuntu/claude_stablecoin/prd.md:432`  
- `/home/ubuntu/claude_stablecoin/prd.md:461`

### 关键证据

- `PROGRESS.md` 的 CP-4 标题是 `LEAP vs LEAP-base`：  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:99`
- 表格对象是 LEAP-base，不是 Block-STM：  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:107`
- Step 1.2 自述也写了“standalone stablecoin benchmark（非 Diem VM）”：  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:11`

### 影响

CP-4 的关键结论不能与 PRD 一一对应，论文叙述会出现“基线对象替换”问题。

---

## MEDIUM-1：PRD Step 2.2 强制产物缺失

### PRD 要求

Step 2.2 要输出：`project/mp3bft/NARWHAL_ANALYSIS.md`  
- `/home/ubuntu/claude_stablecoin/prd.md:507`  
- `/home/ubuntu/claude_stablecoin/prd.md:511`

### 关键证据

- 文件不存在：`/home/ubuntu/claude_stablecoin/mp3bft/NARWHAL_ANALYSIS.md`（已核对）

### 影响

Phase 2 的“仓库组织分析完成”缺乏指定交付物支撑，不满足 PRD 的文档化要求。

---

## MEDIUM-2：实验 2 默认入口与报告口径错位

### 现象

`PROGRESS.md` 和 `REPORT.md` 强调“基于 Narwhal 框架、真实 TCP + Ed25519”。  
但 `experiments/exp2_consensus/run_all.sh` 默认跑的是 `mp3bft_benchmark`（单进程模拟器），而不是 Narwhal 分布式 benchmark 脚本。

### 关键证据

- 默认入口只构建并运行 `mp3bft_benchmark`：  
  - `/home/ubuntu/claude_stablecoin/experiments/exp2_consensus/run_all.sh:13`  
  - `/home/ubuntu/claude_stablecoin/experiments/exp2_consensus/run_all.sh:18`
- `mp3bft_benchmark` 使用内存数据结构 + `thread::sleep` 模拟 RTT：  
  - `/home/ubuntu/claude_stablecoin/mp3bft/src/main.rs:48`  
  - `/home/ubuntu/claude_stablecoin/mp3bft/src/main.rs:82`  
  - `/home/ubuntu/claude_stablecoin/mp3bft/src/main.rs:120`
- 确实存在真实 Narwhal 路径（但不在上述默认脚本里）：  
  - `/home/ubuntu/claude_stablecoin/narwhal/benchmark/run_comparison.py:5`  
  - `/home/ubuntu/claude_stablecoin/narwhal/benchmark/run_comparison.py:113`  
  - `/home/ubuntu/claude_stablecoin/narwhal/benchmark/run_comparison.py:321`

### 影响

复现实验时用户很容易跑错脚本，得到与报告主结论不同性质的数据（模拟 vs 分布式实测），造成可复现性混乱。

---

## MEDIUM-3：共识“TPS 不低于 Tusk”并非全表成立

### PRD 红线

共识层 TPS 必须不低于 Narwhal-Tusk 基线：  
- `/home/ubuntu/claude_stablecoin/prd.md:5`

### 关键证据（来自 PROGRESS 自报表）

- Workers=1 时 MP3 k=4 低于 Tusk：  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:169`
- Workers=10 时 MP3 k=4 低于 Tusk：  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:172`
- Nodes=20 时 MP3 k=4 略低于 Tusk：  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md:180`

### 影响

“全局不低于”结论应改成“多数场景持平或更优”，否则属于表述过度。

---

## 4. 一致性与可信度评估

## 4.1 一致性问题（文档 vs 数据 vs 代码）

1. CP-4 的“通过”与表中单调性数据冲突。  
2. LEAP 优化“已实现”与主路径接入程度冲突。  
3. 实验 2 报告叙述与默认执行脚本冲突。  
4. Step 2.2 勾选完成与产物缺失冲突。

## 4.2 可信度判断

1. 单元测试可信：高（本地复验通过）。  
2. 关键性能结论可信：中低（存在入口错位和对比对象偏移）。  
3. “严格满足 PRD 红线”可信：低（已发现直接冲突项）。

---

## 5. 返工优先级建议（可直接转成任务单）

## P0（立即）

1. 修正 `PROGRESS.md` 中 CP-4 状态与描述，禁止“数据不满足但写 PASSED”。  
2. 明确区分“模拟实验数据”和“Narwhal 分布式实测数据”，并在标题与脚本入口中强制区分。  
3. 补齐 `mp3bft/NARWHAL_ANALYSIS.md`。

## P1（本轮）

1. 将 `domain_plan/hot_delta/backpressure` 真正接入 LEAP 执行主路径。  
2. 重新运行 CP-3/CP-4，并增加自动校验：  
   - 单调性检查（1->4->8->16）  
   - 对比对象必须包含 Block-STM（非 LEAP-base 替代）
3. 更新实验脚本：  
   - `exp2_consensus/run_all.sh` 默认调用 `narwhal/benchmark/run_comparison.py`  
   - 模拟器实验单独命名为 `run_simulator.sh`（避免歧义）

## P2（下一轮）

1. 给每个检查点增加“原始日志 + 解析脚本 + 结果 CSV”的三件套归档。  
2. 在 CI 或 Makefile 中加入“红线守卫”（不满足则失败）。

---

## 6. 可复核证据索引（关键文件）

- PRD 约束：  
  - `/home/ubuntu/claude_stablecoin/prd.md`
- 进度主文档：  
  - `/home/ubuntu/claude_stablecoin/PROGRESS.md`
- LEAP 主路径：  
  - `/home/ubuntu/claude_stablecoin/leap/src/executor.rs`  
  - `/home/ubuntu/claude_stablecoin/leap/src/stablecoin.rs`  
  - `/home/ubuntu/claude_stablecoin/leap/src/config.rs`
- LEAP 三项优化模块：  
  - `/home/ubuntu/claude_stablecoin/leap/src/domain_plan.rs`  
  - `/home/ubuntu/claude_stablecoin/leap/src/hot_delta.rs`  
  - `/home/ubuntu/claude_stablecoin/leap/src/backpressure.rs`
- 实验 2 脚本链：  
  - `/home/ubuntu/claude_stablecoin/experiments/exp2_consensus/run_all.sh`  
  - `/home/ubuntu/claude_stablecoin/mp3bft/src/main.rs`  
  - `/home/ubuntu/claude_stablecoin/narwhal/benchmark/run_comparison.py`

---

## 7. 最终判定

Claude 的工作在“可运行原型”层面有进展，但在“严格满足 PRD”层面存在明显缺口。  
尤其是红线校验、优化接线闭环、实验入口一致性这三项，当前不能视为已完成。

