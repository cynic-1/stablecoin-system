# Requirements v2: 面向稳定币的高性能并行区块链系统 — AI 实现指南

> **目标读者**：AI 编码代理（Claude / Cursor）
> **核心原则**：每完成一个步骤，必须更新进度文件 `PROGRESS.md`；遇到不确定的决策，优先保守实现再迭代优化。
> **⚠️ 绝对红线**：执行引擎必须在增加线程数时 TPS 单调不减（至少到 16 线程）；共识层 TPS 必须不低于 Narwhal-Tusk 基线。
> **⚠️ 前提约束**：LEAP 与 Block-STM（LEAP-base）的性能对比仅在并行执行优于串行执行的工况下有效。必须先通过 CP-1 确定并行可行阈值。

---

## 0. 项目总览与目录结构

### 0.1 研究背景

硕士毕设题目：「面向稳定币的高性能并行区块链系统设计与实现」。系统包含两大创新：

1. **MP3-BFT++ 共识协议**：执行感知的多提议者 BFT，基于数据面/控制面分离 + 双层证书（SlotQC/MacroQC）。
2. **LEAP 并行执行引擎**：在 Block-STM 基础上增加域感知调度、Hot-Delta 热点分片、自适应背压。

两模块通过 **CADO**（Conflict-Aware Deterministic Ordering）接口耦合。

### 0.2 初始目录结构

```
project/
├── narwhal/          # Narwhal-and-Tusk 仓库 (https://github.com/asonnino/narwhal)
│   ├── crypto/       # ed25519-dalek 密码学
│   ├── network/      # tokio 网络层
│   ├── store/        # rocksdb 存储
│   ├── config/       # 委员会/参数配置
│   ├── primary/      # Narwhal primary 节点
│   ├── worker/       # Narwhal worker 节点
│   ├── consensus/    # Tusk / Bullshark / HotStuff 实现
│   └── benchmark/    # 现有 benchmark 框架
│
└── Block-STM/        # Block-STM 官方实现 (https://github.com/danielxiangzl/Block-STM)
    ├── src/
    │   ├── mvmemory.rs     # 多版本内存 (MVMemory)
    │   ├── scheduler.rs    # 调度器
    │   ├── executor.rs     # 执行逻辑
    │   ├── txn.rs          # 交易抽象
    │   └── ...
    └── Cargo.toml
```

### 0.3 最终目标目录结构

```
project/
├── narwhal/              # [已有] 不修改核心代码，仅在 benchmark 层扩展
├── Block-STM/            # [已有] 作为参照和基线，不修改
│
├── leap/                 # [新增] LEAP 执行引擎（fork Block-STM 后改进）
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── mvmemory.rs         # 继承 Block-STM 的 MVMemory，增加 Hot-Delta
│       ├── scheduler.rs        # 继承 Block-STM 调度器，增加域感知 + 背压
│       ├── executor.rs         # 继承 Block-STM 执行器
│       ├── domain_plan.rs      # [新增] 域感知执行计划
│       ├── hot_delta.rs        # [新增] 热点增量分片
│       ├── backpressure.rs     # [新增] 自适应背压
│       ├── cado.rs             # [新增] CADO 排序接口
│       ├── conflict_spec.rs    # [新增] 冲突规约
│       └── stablecoin.rs       # [新增] 稳定币交易模型与 VM
│
├── mp3bft/               # [新增] MP3-BFT++ 共识协议
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── types.rs
│       ├── config.rs
│       ├── data_plane.rs
│       ├── cado.rs
│       ├── control_plane/
│       │   ├── mod.rs
│       │   ├── anti_duplication.rs
│       │   ├── slot_layer.rs
│       │   ├── macro_layer.rs
│       │   └── view_change.rs
│       └── tests/
│
├── experiments/          # [新增] 实验套件（三大类严格分离）
│   ├── exp1_execution/   # 实验一：执行引擎对比
│   ├── exp2_consensus/   # 实验二：共识协议对比
│   └── exp3_e2e/         # 实验三：端到端系统对比
│
└── PROGRESS.md           # [新增] 全局进度追踪文件
```

---

## 1. 进度追踪机制（强制要求）

### 1.1 PROGRESS.md 规范

每完成一个步骤（Step），必须更新 `project/PROGRESS.md`，格式如下：

```markdown
# 项目进度追踪

## 当前阶段
Phase X - Step Y: [步骤名称]

## 已完成步骤

### Phase 1: LEAP 执行引擎
- [x] Step 1.1: 分析 Block-STM 源码结构 (完成时间)
  - Block-STM 核心组件: MVMemory, Scheduler, Executor
  - 关键文件: src/mvmemory.rs (XX行), src/scheduler.rs (XX行)
  - 入口函数: ParallelExecutor::execute_transactions()
  - 性能基线: [记录 Block-STM 原始 benchmark 数据]
- [x] Step 1.2: Fork Block-STM 创建 LEAP crate
  - 修改内容: ...
  - 测试结果: LEAP 无修改版 == Block-STM（TPS 差异 < 2%）
- [ ] Step 1.3: ...（下一步）

### Phase 2: MP3-BFT++ 共识
（待开始）

### Phase 3: 端到端集成
（待开始）

## 关键数据记录
### Block-STM 基线数据
| 线程数 | 无冲突 TPS | Zipf=0.8 TPS | Zipf=1.2 TPS |
|--------|-----------|-------------|-------------|
| 1      |           |             |             |
| 4      |           |             |             |
| 16     |           |             |             |

### Narwhal-Tusk 基线数据
| 节点数 | TPS    | Latency p50 | Latency p99 |
|--------|--------|-------------|-------------|
| 4      |        |             |             |
| 10     |        |             |             |

## 遇到的问题与解决方案
1. [问题描述] → [解决方案]
```

### 1.2 检查点机制

在以下关键节点，必须停下来验证数据，再继续：

| 检查点 | 验证内容 | 通过标准 |
|--------|---------|---------|
| CP-1 | Block-STM 基线数据采集 | 多线程 TPS > 单线程 TPS |
| CP-2 | LEAP fork 等价性 | LEAP 无修改版 ≈ Block-STM（差异 < 5%） |
| CP-3 | LEAP 各优化独立验证 | 每个优化开启后 TPS ≥ 关闭时 |
| CP-4 | LEAP 完整版验证 | 所有 Zipf 参数下 TPS ≥ Block-STM |
| CP-5 | Narwhal-Tusk 基线数据 | 本地 benchmark 成功运行 |
| CP-6 | MP3-BFT++ 基础对比 | TPS ≥ Narwhal-Tusk |
| CP-7 | 端到端集成验证 | 交易从提交到执行完成正确 |

---

## 2. Phase 1: LEAP 执行引擎（基于 Block-STM 改进）

> **核心策略**：不要从零实现执行引擎！先完整理解 Block-STM 代码，确认其 benchmark 在本地可运行且结果合理，然后 fork 一份作为 LEAP 基础，逐个叠加优化点。每叠加一个优化，都要验证：(a) 正确性不退化，(b) 性能不退化。
>
> **⚠️ 前提条件**：LEAP 与 Block-STM（LEAP-base）的性能对比**仅在并行执行优于串行执行的工况下才有意义**。当单笔交易计算开销极低（如 0μs，纯内存操作）时，串行执行因无同步开销可达数亿 TPS，而任何并行引擎都受限于原子操作 / MVHashMap 同步开销，TPS 远低于串行——此时对比并行引擎之间的差异毫无价值。因此：
> - **CP-1 必须首先确定「并行可行阈值」**：找到最低的单笔交易开销（crypto overhead），使得多线程并行 TPS > 单线程串行 TPS。
> - **CP-4 的对比仅在此阈值以上的开销水平进行**。低于此阈值的数据点仅作参考，不纳入性能对比结论。
> - 推荐的实验开销范围：10μs–100μs（对应真实区块链中签名验证 / Merkle proof 等操作）。

### Step 1.1: 深入分析 Block-STM 源码

**任务**：阅读 `Block-STM/` 全部源码，输出分析报告。

**输出文件**：`project/leap/BLOCK_STM_ANALYSIS.md`

需要记录的信息：
```markdown
# Block-STM 源码分析报告

## 1. 整体架构
- 入口函数和调用链
- 核心组件及其职责
- 线程模型（几个线程？如何分工？）

## 2. 关键数据结构
- MVMemory 的结构（版本链如何组织？用什么容器？）
- Scheduler 的状态机（交易经历哪些状态？）
- ReadSet / WriteSet 的表示

## 3. 核心算法流程
- 投机执行流程（对应本文 Algorithm 4.3）
- 验证流程（对应本文 Algorithm 4.4）
- 撤销重试流程（对应本文 Algorithm 4.5）
- 调度策略（execIdx / valIdx 如何推进？）

## 4. 交易抽象
- Transaction trait 接口定义
- 现有 benchmark 中交易是如何构造的？
- 读写集如何传递？

## 5. Benchmark 分析
- 现有 benchmark 如何运行？参数有哪些？
- 冲突是如何制造的？（随机读写？Zipf？）
- 指标如何采集？

## 6. LEAP 改进切入点映射
| LEAP 优化点 | Block-STM 中对应的代码位置 | 改动方式 |
|-------------|--------------------------|---------|
| 域感知调度   | scheduler.rs 的 next_task() | 替换选择策略 |
| Hot-Delta   | mvmemory.rs 的 write()    | 增加分片写入路径 |
| 背压控制    | scheduler.rs 的窗口限制    | 增加动态调整 |
| CADO 输入   | 交易序列的排列方式          | 增加预排序步骤 |
```

### Step 1.2: 运行 Block-STM 基线 Benchmark

**任务**：在本地运行 Block-STM 原始 benchmark，采集基线数据。

**必须采集的数据**（记录到 PROGRESS.md）：

```
Block-STM Baseline Benchmarks
=============================
交易数: 10000
账户数: 1000 (或 Block-STM 默认值)

| 线程数 | 无冲突 TPS | 低冲突 TPS | 中冲突 TPS | 高冲突 TPS |
|--------|-----------|-----------|-----------|-----------|
| 1      |           |           |           |           |
| 2      |           |           |           |           |
| 4      |           |           |           |           |
| 8      |           |           |           |           |
| 16     |           |           |           |           |
| 32     |           |           |           |           |
```

**检查点 CP-1**：确认多线程 TPS > 单线程 TPS。
- 在多个开销水平（0μs, 1μs, 3μs, 10μs, 50μs, 100μs）下分别运行串行和多线程基线。
- **确定并行可行阈值**：找到最低开销水平 $\tau$，使得 $\text{TPS}_{\text{parallel}}(t \geq 4) > \text{TPS}_{\text{serial}}$。
- 记录此阈值到 PROGRESS.md。后续 CP-3/CP-4 的引擎对比仅在 $\geq \tau$ 的开销水平下进行。
- 如果在所有开销水平下多线程都慢于串行，先排查原因（可能是 benchmark 参数不当），不要继续。

### Step 1.3: Fork Block-STM → LEAP crate

**任务**：将 Block-STM 源码复制到 `project/leap/`，调整 crate 名称和模块组织，确保编译通过、benchmark 结果与原版一致。

**具体操作**：
1. `cp -r Block-STM/src/* leap/src/`
2. 修改 `Cargo.toml` 中的 crate name 为 `leap`
3. 不改动任何逻辑代码
4. 运行同样的 benchmark，对比结果

**检查点 CP-2**：LEAP（无修改版）与 Block-STM 在同参数下 TPS 差异 < 5%。

### Step 1.4: 添加稳定币交易模型

**任务**：在 LEAP 中实现稳定币场景的 Transaction trait。

**文件**：`leap/src/stablecoin.rs`

```rust
/// 稳定币交易类型
pub enum StablecoinTxType {
    Transfer { sender: u64, receiver: u64, amount: u64 },
    Mint { to: u64, amount: u64 },
    Burn { from: u64, amount: u64 },
}

/// 实现 Block-STM 的 Transaction trait
/// 关键：读写集必须正确声明
impl Transaction for StablecoinTx {
    fn execute(&self, view: &MVMemoryView) -> ExecutionResult {
        match &self.tx_type {
            Transfer { sender, receiver, amount } => {
                // 1. 读 sender 余额 & nonce
                // 2. 检查余额充足
                // 3. 写 sender 余额（扣减）
                // 4. 读 receiver 余额
                // 5. 写 receiver 余额（增加）
                // 6. 写 sender nonce + 1
            }
            // Mint, Burn 类似...
        }
    }
}
```

**同时实现稳定币工作负载生成器**：

```rust
pub struct StablecoinWorkloadGenerator {
    num_accounts: usize,
    hotspot_config: HotspotConfig,
}

pub enum HotspotConfig {
    /// 均匀随机：所有账户等概率被选为收款方
    Uniform,
    /// Zipf 分布：alpha 越大热点越集中
    Zipf { alpha: f64 },
    /// 显式热点：指定比例的交易指向少数热点账户
    Explicit { num_hotspots: usize, hotspot_ratio: f64 },
}

impl StablecoinWorkloadGenerator {
    /// 生成 n 笔交易
    pub fn generate(&self, n: usize) -> Vec<StablecoinTx>;
}
```

**验证**：用稳定币工作负载运行 LEAP（此时还是纯 Block-STM 逻辑），确认多线程加速正常。

### Step 1.5: 实现 CADO 排序接口

**文件**：`leap/src/cado.rs`

```rust
/// 冲突感知确定性排序
/// 将交易集合按冲突域分组、域内按 (sender, nonce) 排序
/// 使同域交易在序列中相邻，为后续域感知调度创造条件
pub fn cado_ordering(txs: &mut Vec<StablecoinTx>) {
    // 1. 按 conflict_domain 的哈希值分组排序（确定性）
    // 2. 同域内按 (sender, nonce) 排序
    // 3. 同 (sender, nonce) 按 tx_hash 仲裁
}
```

**验证**：
- 确定性测试：相同输入多次运行 → 相同输出
- 用 CADO 排序后的序列运行 LEAP，结果与串行执行一致

### Step 1.6: 实现域感知调度（优化点 1）

**文件**：`leap/src/domain_plan.rs`

**改动思路**：修改 Block-STM 调度器的 `next_task()` 函数，在选择下一个投机执行的交易时，优先选择与当前验证域段不同域的交易。

```rust
pub struct DomainSegment {
    pub start: usize,
    pub end: usize,       // exclusive
    pub domain: u64,      // 冲突域标识的哈希
    pub write_keys: HashSet<u64>,
}

pub struct DomainPlan {
    pub segments: Vec<DomainSegment>,
    pub par_bound: Vec<bool>,  // 是否可与前一段并行
}

/// 从 CADO 排序后的序列构建域感知计划
pub fn build_domain_plan(txs: &[StablecoinTx], l_max: usize) -> DomainPlan;
```

**对 Scheduler 的修改**：
- 在 `next_task()` 中增加域感知逻辑：如果当前域段内有未验证交易，优先验证
- 跨域段且 `par_bound[j] == true` 时允许并行投机

**验证**：
- 串行等价性：开启域感知后执行结果 == 串行执行结果
- **CP-3 检查**：在 Zipf ≥ 0.8 的场景下，域感知开启 TPS ≥ 关闭时 TPS

### Step 1.7: 实现 Hot-Delta 热点分片（优化点 2）

**文件**：`leap/src/hot_delta.rs`

**这是最关键的优化，必须极其谨慎**。

**改动思路**：对于被标识为热点的收款账户余额键，写入时不直接写 `bal(account)`，而是写入 `delta(account, shard_id)`。读取时聚合所有分片。

```rust
pub struct HotDeltaManager {
    shard_counts: HashMap<u64, usize>,  // account -> P(a)
    theta_1: usize,  // 热度阈值 1 (默认 10)
    theta_2: usize,  // 热度阈值 2 (默认 50)
}

impl HotDeltaManager {
    /// 预扫描交易序列，统计账户热度
    pub fn detect_hotspots(&mut self, txs: &[StablecoinTx]);

    /// 获取账户的分片数
    pub fn shard_count(&self, account: u64) -> usize;

    /// 将 bal(account) 的写操作转换为 delta(account, p) 的写操作
    /// p = hash(tx_hash) % P(account)
    pub fn delta_key(account: u64, tx_hash: u64, shard_count: usize) -> StateKey;

    /// 读取完整余额：base(account) + sum(delta(account, 0..P))
    pub fn read_balance(account: u64, view: &MVMemoryView, shard_count: usize) -> u64;
}
```

**对 MVMemory 和 Executor 的修改**：
- 在 `execute()` 中，对热点收款账户的余额写入，改用 `delta_key` 路径
- 在 `execute()` 中，对热点账户的余额读取，改用 `read_balance` 聚合路径
- **注意**：发送方余额的扣减仍然直接写 `bal(sender)`，因为余额充足性检查不可交换

**验证**：
- 语义等价性（定理 4.2）：Hot-Delta 模式下最终余额 == 非 Hot-Delta 模式
- **属性测试**：随机生成 10000+ 组交易，验证分片聚合结果 == 直接累加
- **CP-3 检查**：在高热点场景（单账户 50%+ 收款）下，Hot-Delta TPS > 无 Hot-Delta TPS

### Step 1.8: 实现自适应背压（优化点 3）

**文件**：`leap/src/backpressure.rs`

**改动思路**：动态调整投机窗口 W = execIdx - valIdx 的上限。

```rust
pub struct BackpressureController {
    w: usize,           // 当前窗口大小
    w_min: usize,       // 4
    w_max: usize,       // 64
    gamma_down: f64,    // 0.8
    gamma_up: f64,      // 1.2
    eta_abort: f64,     // 0.1
    eta_wait: f64,      // 0.1
}

impl BackpressureController {
    /// 根据上一块的统计调整窗口
    pub fn adjust(&mut self, stats: &BlockExecStats) {
        if stats.abort_rate > self.eta_abort || stats.wait_rate > self.eta_wait {
            self.w = max(self.w_min, (self.w as f64 * self.gamma_down) as usize);
        } else if stats.abort_rate < self.eta_abort / 2.0 && stats.wait_rate < self.eta_wait / 2.0 {
            self.w = min(self.w_max, (self.w as f64 * self.gamma_up) as usize);
        }
    }
}
```

**对 Scheduler 的修改**：在 `next_task()` 中增加窗口检查：`if exec_idx - val_idx > W { return None; }`

**验证**：
- 低冲突时窗口应自动扩大，高冲突时自动收缩
- CP-3 检查：极高冲突场景下，背压开启 TPS ≥ 关闭时 TPS

### Step 1.9: LEAP 完整版集成与验证

**任务**：所有优化点同时开启，运行全面 benchmark。

**检查点 CP-4**（最关键！）：

> **⚠️ 前提**：以下对比仅在 **并行可行阈值以上** 的开销水平进行（参见 CP-1）。
> 低于此阈值的开销水平（如 0μs）串行执行远快于并行执行，比较并行引擎之间的差异没有意义。
> 推荐对比范围：10μs–100μs（对应真实签名验证 / Merkle 操作）。

```
LEAP vs Block-STM (LEAP-base) 对比数据
======================================
交易数: 10000, 账户数: 1000, 线程数: 1/4/8/16
开销水平: 仅取并行可行阈值以上（推荐 10μs, 50μs, 100μs）

前提验证（每个开销水平）：
| 开销    | Serial TPS | Parallel 4t TPS | 并行 > 串行? |
|---------|-----------|-----------------|-------------|
| 10μs    |           |                 | ✅ / ❌      |
| 50μs    |           |                 | ✅ / ❌      |
| 100μs   |           |                 | ✅ / ❌      |

→ 仅对「并行 > 串行」的行进行以下 LEAP vs LEAP-base 对比。

场景 1: 均匀分布（无冲突）
| 引擎       | 1线程 | 4线程 | 8线程 | 16线程 |
|------------|-------|-------|-------|--------|
| Serial     |       |  N/A  |  N/A  |  N/A   |
| LEAP-base  |       |       |       |        |
| LEAP       |       |       |       |        |

场景 2: Zipf α=0.8（中等冲突）
（同上格式）

场景 3: Zipf α=1.2（高冲突）
（同上格式）

场景 4: 显式热点（50% 交易→单一账户）
（同上格式）

场景 5: 显式热点（90% 交易→单一账户）
（同上格式）
```

**必须满足的条件**（仅在并行可行阈值以上的开销水平验证）：
1. ✅ LEAP 在所有场景下 TPS ≥ LEAP-base（允许低冲突下持平）
2. ✅ LEAP 在高冲突和热点场景下明显优于 LEAP-base
3. ✅ 增加线程数时 TPS 单调不减（至少到 16 线程）
4. ✅ LEAP 执行结果与串行执行完全一致（正确性）

**如果 CP-4 不通过**：
- 若 LEAP 整体比 LEAP-base 慢 → 逐个关闭优化点，定位性能退化源
- 若多线程反而更慢 → 检查锁争用、false sharing、原子操作开销
- 若仅某些冲突度下劣于 LEAP-base → 调整背压参数或 Hot-Delta 阈值
- **不要继续进入 Phase 2，直到 CP-4 完全通过**

---

## 3. Phase 2: MP3-BFT++ 共识协议（基于 Narwhal 仓库）

> **核心策略**：先跑通 Narwhal-Tusk 的本地 benchmark 采集基线数据，然后在 narwhal 仓库的代码组织模式下实现 MP3-BFT++，复用其密码学、网络、存储基础设施。

### Step 2.1: 运行 Narwhal-Tusk 本地 Benchmark

**任务**：按 narwhal 仓库的 README 指引，运行本地 benchmark。

**参考命令**（以仓库文档为准）：
```bash
cd narwhal
# 按 README 说明编译和运行 benchmark
# 通常涉及：fab local  或  cargo run --release --bin node ...
```

**必须采集的基线数据**（记录到 PROGRESS.md）：

```
Narwhal-Tusk Baseline Benchmarks
=================================

| 节点数 | 故障数 | 输入速率(tx/s) | TPS (committed) | Latency p50(ms) | Latency p99(ms) |
|--------|--------|---------------|-----------------|-----------------|-----------------|
| 4      | 0      | 50000         |                 |                 |                 |
| 4      | 0      | 100000        |                 |                 |                 |
| 4      | 0      | 200000        |                 |                 |                 |
| 4      | 1      | 100000        |                 |                 |                 |
| 10     | 0      | 100000        |                 |                 |                 |
| 10     | 3      | 100000        |                 |                 |                 |
```

**检查点 CP-5**：本地 benchmark 成功运行并获得合理数据（TPS > 0 且延迟非异常值）。

### Step 2.2: 分析 Narwhal 仓库代码组织

**任务**：分析仓库中 Tusk/Bullshark/HotStuff 的实现方式，了解如何新增一个共识协议。

**输出文件**：`project/mp3bft/NARWHAL_ANALYSIS.md`

需要记录：
```markdown
# Narwhal 仓库代码组织分析

## 1. 共识协议接口
- consensus 模块的 trait 定义
- 新共识协议需要实现哪些接口？
- 输入是什么？（从 Narwhal DAG 获取的 certificate？还是原始交易？）
- 输出是什么？（提交的交易序列？）

## 2. 与 Narwhal 数据面的交互
- Worker 如何向 Primary 传递 batch？
- Primary 如何形成 Certificate（对应我们的 AvailCert）？
- Certificate 如何传递给 Consensus 模块？

## 3. Benchmark 框架
- benchmark 客户端如何发送交易？
- 如何测量 TPS 和延迟？
- 如何配置节点数、故障数、输入速率？
- MP3-BFT++ 需要如何接入才能使用同一 benchmark？

## 4. 关键发现 & 适配方案
- MP3-BFT++ 的数据面 vs Narwhal 的数据面：哪些可复用？
- MP3-BFT++ 的控制面如何映射到 narwhal 的模块结构？
```

### Step 2.3: 实现 MP3-BFT++ 数据结构

**文件**：`mp3bft/src/types.rs`

定义所有协议消息和数据结构（详细定义见论文 3.2 节）：

- `Batch`、`AvailCert`（数据面）
- `BlockletProposal`、`SlotVote`、`SlotQC`（槽级认证层）
- `MacroHeader`、`SlotEntry`、`MacroVote`、`MacroQC`（宏块链终局层）
- `NewViewMessage`（视图切换）

**关键**：尽量复用 narwhal 仓库的 `crypto` 类型（`PublicKey`、`Signature`、`Digest` 等）。

### Step 2.4: 实现数据面

**文件**：`mp3bft/src/data_plane.rs`

**设计决策**：MP3-BFT++ 的数据面功能与 Narwhal 的 Worker 层高度重合（都是批次广播 + 2f+1 签名确认可用性）。有两种策略：

- **策略 A（推荐）**：直接复用 Narwhal 的 Worker 层作为数据面，将 Narwhal Certificate 视为 AvailCert。
- **策略 B**：独立实现数据面，不依赖 Narwhal Worker。

选择策略 A 可以确保与 Narwhal-Tusk 的公平对比（相同的数据面），差异仅在控制面。

### Step 2.5: 实现控制面 — 反重复分配

**文件**：`mp3bft/src/control_plane/anti_duplication.rs`

```rust
/// 桶分配（Algorithm 3.1）
pub fn assign_buckets(view: u64, n_buckets: usize, k_slots: usize) -> Vec<BucketRange>;

/// 桶合规检查（验证者投票前的硬约束）
pub fn check_bucket_compliance(proposal: &BlockletProposal, range: &BucketRange, n_b: usize) -> bool;

/// 槽内唯一性检查
pub fn check_intra_slot_unique(proposal: &BlockletProposal) -> bool;
```

**单元测试**：
- 不同视图下桶分配正确轮换
- 跨槽交易被正确拒绝
- 槽内重复 (sender, nonce) 被正确拒绝

### Step 2.6: 实现控制面 — 槽级认证

**文件**：`mp3bft/src/control_plane/slot_layer.rs`

三个角色：SlotProposer（Algorithm 3.2）、SlotValidator（Algorithm 3.3）、SlotCollector（Algorithm 3.4）。

**关键实现要点**：
- 提议者选择：`H(view || slot || epoch_seed) mod n`
- 验证者投票前必须通过桶合规 + 槽内唯一性检查
- 收集者聚合 ≥2f+1 投票形成 SlotQC
- 备份收集者容错（Algorithm 3.8）

### Step 2.7: 实现控制面 — 宏块链终局

**文件**：`mp3bft/src/control_plane/macro_layer.rs`

- MacroLeader（Algorithm 3.5）：收集窗口内聚合 SlotQC → 组装 MacroHeader → 广播
- MacroValidator：验证 MacroHeader → 发送 MacroVote
- MacroCollector：聚合 ≥2f+1 MacroVote → MacroQC
- **3-chain commit 规则**：连续认证链 $B_{h-2} \leftarrow B_{h-1} \leftarrow B_h$ → 提交 $B_{h-2}$
- **锁规则**：投票前检查 `parent_qc.height >= locked_qc.height`

### Step 2.8: 实现视图切换

**文件**：`mp3bft/src/control_plane/view_change.rs`（Algorithm 3.7）

- 超时触发 → 发送 NEW_VIEW
- 新领导者收集 ≥2f+1 NEW_VIEW → 选最高 highQC → 恢复

### Step 2.9: 实现 CADO（共识层侧）

**文件**：`mp3bft/src/cado.rs`

复用 LEAP 中 Step 1.5 的 CADO 逻辑，但作为共识模块的输出接口：宏块提交后 → 解包交易集合 → CADO 排序 → 输出 $\pi_h$。

### Step 2.10: 接入 Narwhal Benchmark 框架

**任务**：将 MP3-BFT++ 作为一种新的共识协议接入仓库的 benchmark，能够与 Tusk、Bullshark、HotStuff 在同一框架下对比。

**检查点 CP-6**：

```
MP3-BFT++ vs Narwhal-Tusk 对比数据
====================================
（使用相同数据面、相同网络条件、相同输入速率）

| 协议        | 节点数 | 并行参数 | TPS     | Latency p50 | Latency p99 |
|-------------|--------|---------|---------|-------------|-------------|
| Tusk        | 4      | N/A     |         |             |             |
| Bullshark   | 4      | N/A     |         |             |             |
| HotStuff    | 4      | N/A     |         |             |             |
| MP3-BFT++   | 4      | k=4     |         |             |             |
| MP3-BFT++   | 4      | k=8     |         |             |             |
| MP3-BFT++   | 4      | k=16    |         |             |             |
```

**必须满足**：
1. ✅ MP3-BFT++ 在某个 k 值下 TPS ≥ Tusk
2. ✅ TPS 随 k 增大呈现增长趋势（验证理论预测的线性扩展）
3. ✅ 无安全性违规（不出现同高度冲突提交）

**如果 CP-6 不通过**：
- 若 TPS 远低于 Tusk → 检查控制面瓶颈（是否 MacroLeader 成为单点？）
- 若 TPS 不随 k 增长 → 检查槽级认证是否真正并行化
- **不要继续进入 Phase 3，直到 CP-6 通过**

---

## 4. Phase 3: 端到端集成与实验

> **前提**：Phase 1（LEAP CP-4 通过）和 Phase 2（MP3-BFT++ CP-6 通过）均已完成。

### Step 3.1: 端到端集成

**任务**：将 MP3-BFT++ 共识 + CADO + LEAP 执行串联，形成完整的稳定币交易处理管道。

```
客户端提交 StablecoinTx
    → MP3-BFT++ 数据面（批次广播 + AvailCert）
    → MP3-BFT++ 控制面（SlotQC + MacroQC + 3-chain commit）
    → CADO 排序（派生 π_h）
    → LEAP 执行（并行执行 + 状态更新）
    → 返回执行结果
```

**检查点 CP-7**：
- 交易从提交到执行完成，最终状态正确
- 多节点状态一致（所有诚实节点的 StateRoot 相同）

### Step 3.2: 端到端实验设计

端到端实验旨在模拟稳定币真实运行场景，采集完整系统的性能数据。

**实验 E2E-1：吞吐量-延迟曲线**
- 固定 n=4, k=8, 16 执行线程
- 逐步增加输入速率，找到饱和点
- 对比：(MP3-BFT++ + LEAP) vs (Narwhal-Tusk + Serial) vs (HotStuff + Serial)
- 指标：端到端 TPS、端到端延迟

**实验 E2E-2：不同冲突模式**
- 固定输入速率为 80% 饱和点
- 变量：均匀分布 / Zipf α=0.8 / Zipf α=1.2 / 显式热点 50%
- 对比：同上
- 指标：TPS、延迟、执行重试次数

**实验 E2E-3：节点扩展性**
- 变量：n ∈ {4, 10, 20}
- 固定冲突模式
- 指标：TPS 随节点数的变化

---

## 5. 实验套件详细规格

### 5.1 实验一（执行引擎）目录结构

```
experiments/exp1_execution/
├── README.md              # 实验说明
├── run_all.sh             # 一键运行所有实验
├── bench_block_stm.rs     # Block-STM 基线 benchmark
├── bench_leap.rs          # LEAP benchmark
├── bench_serial.rs        # 串行执行 benchmark
├── workload.rs            # 稳定币工作负载生成
├── results/
│   ├── raw/               # 原始数据 CSV
│   └── plots/             # 图表
└── plot.py                # 图表生成脚本
```

### 5.2 实验一子实验清单

| 编号 | 实验名 | X 轴 | 系列 | 指标 |
|------|--------|------|------|------|
| 1A | 线程扩展性（无冲突） | 线程数 1-32 | Serial/Block-STM/LEAP | TPS |
| 1B | 线程扩展性（高冲突） | 线程数 1-32 | Serial/Block-STM/LEAP | TPS |
| 1C | 冲突敏感度 | Zipf α 0-2.0 | Block-STM/LEAP | TPS |
| 1D | 热点集中度 | 热点比例 10%-90% | Block-STM/LEAP/LEAP-noHotDelta | TPS |
| 1E | LEAP 消融 | Zipf α 0/0.8/1.2/2.0 | LEAP-full/noDomain/noHotDelta/noBackpressure/Block-STM | TPS |
| 1F | 块大小影响 | 块大小 1K-100K | Block-STM/LEAP | TPS |
| 1G | 重试次数分析 | Zipf α 0-2.0 | Block-STM/LEAP | 平均重试次数 |

### 5.3 实验二（共识协议）目录结构

```
experiments/exp2_consensus/
├── README.md
├── run_all.sh
├── configs/               # 不同节点数的配置文件
├── results/
│   ├── raw/
│   └── plots/
└── plot.py
```

### 5.4 实验二子实验清单

| 编号 | 实验名 | X 轴 | 系列 | 指标 |
|------|--------|------|------|------|
| 2A | TPS vs 节点数 | n ∈ {4,10,20,50} | Tusk/Bullshark/HotStuff/MP3-BFT++ | TPS |
| 2B | 延迟 vs 节点数 | n ∈ {4,10,20,50} | 同上 | Latency p50/p99 |
| 2C | TPS vs 并行槽数 k | k ∈ {1,2,4,8,16} | MP3-BFT++ only | TPS |
| 2D | 容错影响 | 故障数 0/1/f | Tusk/MP3-BFT++ | TPS |
| 2E | 控制面开销 | k ∈ {1,2,4,8,16} | MP3-BFT++ only | 控制面带宽(MB/s) |

### 5.5 实验三（端到端）目录结构

```
experiments/exp3_e2e/
├── README.md
├── run_all.sh
├── results/
│   ├── raw/
│   └── plots/
└── plot.py
```

### 5.6 实验三子实验清单

| 编号 | 实验名 | X 轴 | 系列 | 指标 |
|------|--------|------|------|------|
| 3A | 吞吐-延迟曲线 | 输入速率 | MP3-BFT+++LEAP / Tusk+Serial / HotStuff+Serial | TPS & Latency |
| 3B | 冲突模式影响 | 冲突模式 | 同上 | TPS |
| 3C | 节点扩展性 | n ∈ {4,10,20} | 同上 | TPS |

### 5.7 图表生成规范

所有图表使用 Python matplotlib 生成，统一风格：

```python
import matplotlib.pyplot as plt

# 统一风格
plt.rcParams.update({
    'font.size': 14,
    'figure.figsize': (8, 5),
    'axes.grid': True,
    'grid.alpha': 0.3,
})

# 颜色方案
COLORS = {
    'Serial':     '#999999',
    'Block-STM':  '#4472C4',
    'LEAP':       '#ED7D31',
    'LEAP-full':  '#ED7D31',
    'Tusk':       '#4472C4',
    'Bullshark':  '#A5A5A5',
    'HotStuff':   '#999999',
    'MP3-BFT++':  '#ED7D31',
}

# 线型
LINESTYLES = {
    'Serial':    '--',
    'Block-STM': '-',
    'LEAP':      '-',
}
```

---

## 6. 关键设计细节（供实现参考）

### 6.1 MP3-BFT++ 配置参数

```rust
pub struct MP3BFTConfig {
    pub k_slots: usize,            // 并行槽数，推荐 4-16
    pub n_buckets: usize,          // 桶数量，≥ 10k，取 2 的幂
    pub m_max: usize,              // 每槽引用 AC 上限，推荐 16-64
    pub s_batch: usize,            // 批次大小，推荐 500-2000
    pub delta_slot: Duration,      // SlotQC 收集窗口
    pub delta_col: Duration,       // 收集者超时
    pub t_initial: Duration,       // 初始视图超时
    pub t_max: Duration,           // 最大视图超时 60s
    pub rho: f64,                  // 退避系数 1.5
    pub ordering_rule_id: u32,     // CADO 版本号
}
```

### 6.2 LEAP 配置参数

```rust
pub struct LeapConfig {
    pub num_workers: usize,         // 工作线程数
    pub w_initial: usize,           // 初始投机窗口 32
    pub w_min: usize,               // 最小窗口 4
    pub w_max: usize,               // 最大窗口 64
    pub l_max: usize,               // 域段大小上限 256
    pub w_scan: usize,              // 弱独立检测窗口 8
    pub theta_1: usize,             // 热度阈值 1 = 10
    pub theta_2: usize,             // 热度阈值 2 = 50
    pub p_max: usize,               // 最大分片数 8
    pub enable_domain_aware: bool,  // 是否启用域感知（消融实验用）
    pub enable_hot_delta: bool,     // 是否启用 Hot-Delta（消融实验用）
    pub enable_backpressure: bool,  // 是否启用背压（消融实验用）
}
```

### 6.3 CADO 排序规则（Algorithm 3.6）

```
输入：已提交宏块的交易集合 T_h（无序）
输出：确定性全序 π_h

1. 按 (sender, nonce) 全局去重
2. 按 conflict_domain 分组
3. 域排序：按 H(domain_id) 升序
4. 域内排序：先按 (sender, nonce)，再按 tx_hash 仲裁
5. 依次连接各域 → π_h
```

### 6.4 稳定币交易冲突规约（表 4.1）

| 交易类型 | 冲突域 d(T) | 声明读集 | 声明写集 |
|---------|------------|---------|---------|
| Transfer(s,r,amt) | receiver r | {bal(s), bal(r), nonce(s)} | {bal(s), bal(r), nonce(s)} |
| Mint(to,amt) | target to | {nonce(minter), bal(to)} | {bal(to), nonce(minter), totalSupply} |
| Burn(from,amt) | source from | {nonce(burner), bal(from)} | {bal(from), nonce(burner), totalSupply} |
| Freeze(acct) | acct | {nonce(admin), frozen(acct)} | {frozen(acct), nonce(admin)} |

---

## 7. 正确性验证清单

每个检查项在实现对应模块后必须验证通过：

### 执行引擎
- [ ] 串行等价性：`LEAP.execute(π_h) == Serial.execute(π_h)`
- [ ] Hot-Delta 语义等价：分片聚合 == 直接累加（属性测试 ≥ 10000 用例）
- [ ] CADO 确定性：相同输入多次运行 → 相同输出
- [ ] 有界重试：任意交易 incarnation ≤ min(i+1, W+1)
- [ ] 线程安全：TSAN 无数据竞争报告

### 共识协议
- [ ] 宏块内无重复交易：π_h 中每个 (sender, nonce) 至多一次
- [ ] 安全性：不存在同高度冲突提交
- [ ] 桶合规硬约束：不合规提议无法形成 SlotQC
- [ ] 活性：视图切换后恢复正常出块

### 端到端
- [ ] 多节点状态一致：所有诚实节点 StateRoot 相同
- [ ] 交易不丢失：所有已提交交易最终被执行

---

## 8. 故障排查指南

### 执行引擎常见问题

| 症状 | 可能原因 | 解决方案 |
|------|---------|---------|
| 多线程 TPS < 单线程 | 锁争用 / false sharing | 检查 MVMemory 的并发容器；对齐缓存行 |
| 多线程 TPS < 单线程 | 原子操作竞争 | 减少 CAS 频率；增大批次粒度 |
| 高冲突下 TPS 塌缩 | 投机窗口过大导致无效重试 | 降低 W_max 或启用背压 |
| Hot-Delta 后结果不一致 | 分片读写逻辑错误 | 对照 Algorithm 4.6 逐行检查 |
| 域感知调度后更慢 | 域段划分不当导致串行化 | 检查 l_max 参数；验证 par_bound 计算 |

### 共识协议常见问题

| 症状 | 可能原因 | 解决方案 |
|------|---------|---------|
| TPS 远低于 Tusk | MacroLeader 成为瓶颈 | 检查是否真正并行处理 k 个槽 |
| TPS 不随 k 增长 | 槽级认证未并行化 | 检查是否为 k 个 SlotCollector 分配独立 task |
| 出现冲突提交 | 锁规则实现错误 | 对照定理 3.2 证明检查投票条件 |
| 视图切换后卡住 | NEW_VIEW 收集逻辑错误 | 检查 QC* 选择是否取最高高度 |

---

## 9. 工作流程总结

```
Phase 1: LEAP 执行引擎
  Step 1.1  分析 Block-STM 源码 → BLOCK_STM_ANALYSIS.md
  Step 1.2  Block-STM 基线 benchmark → CP-1 ✓
  Step 1.3  Fork → LEAP crate → CP-2 ✓
  Step 1.4  稳定币交易模型
  Step 1.5  CADO 排序
  Step 1.6  域感知调度 → CP-3 ✓
  Step 1.7  Hot-Delta 分片 → CP-3 ✓
  Step 1.8  自适应背压 → CP-3 ✓
  Step 1.9  LEAP 完整验证 → CP-4 ✓✓✓（必须通过！）
  ────── 产出：experiments/exp1_execution/ 全部数据与图表 ──────

Phase 2: MP3-BFT++ 共识
  Step 2.1  Narwhal-Tusk 基线 benchmark → CP-5 ✓
  Step 2.2  Narwhal 代码分析 → NARWHAL_ANALYSIS.md
  Step 2.3  数据结构定义
  Step 2.4  数据面（复用 Narwhal Worker）
  Step 2.5  反重复分配
  Step 2.6  槽级认证
  Step 2.7  宏块链终局
  Step 2.8  视图切换
  Step 2.9  CADO（共识侧）
  Step 2.10 接入 benchmark → CP-6 ✓✓✓（必须通过！）
  ────── 产出：experiments/exp2_consensus/ 全部数据与图表 ──────

Phase 3: 端到端集成
  Step 3.1  集成管道 → CP-7 ✓
  Step 3.2  端到端实验
  ────── 产出：experiments/exp3_e2e/ 全部数据与图表 ──────
```

**每完成一个 Step → 更新 PROGRESS.md → 确认检查点 → 继续下一步。**

**遇到检查点不通过 → 立即停下来排查 → 在 PROGRESS.md 记录问题与解决方案 → 修复后重新验证 → 通过后才继续。**
