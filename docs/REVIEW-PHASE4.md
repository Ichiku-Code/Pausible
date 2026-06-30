## 代码审查报告 — Phase 4（含 Phase 1–3 复核）

审查时间：2026-07-01 | 审查范围：PHASE 1–4（全部源码 + 文档对比）

### 测试与静态检查状态

- 单元测试 + 集成测试：185 passed, 0 failed（1 ignored 为 tcp_echo_roundtrip 因沙箱无网络权限，非代码问题）
- Clippy pedantic：零警告

### REVIEW-PHASE3.md 遗留问题修复确认

上一份 review 标记的 9 个问题中，8 个已修复：

| 编号 | 问题 | 状态 |
|------|------|------|
| P0-#1 | Append 模式截断文件 | 已修复 — `write_file_handle` 根据 `mode` 选择 `.append(true)` 或 `.truncate(true)` |
| P0-#5 | `value_to_bytes` 对 String 返回空 | 已修复 — 接受 `&Heap` 参数，通过 GC 引用解析实际字节 |
| P1-#2 | `write_file_handle` 非 File handle 静默返回 | 已修复 — fallthrough 返回 `VmError::IoError("handle is not a File")` |
| P1-#3 | `write_stdout`/`write_stderr` 返回值不一致 | 已修复 — 返回类型改为 `Result<(), VmError>` |
| P3-#6 | 双重 `#[allow(dead_code)]` | 已修复 — 仅剩一个，且该注解必要正确（见 P2-#3 分析） |
| P3-#7 | PHASE3.md §3.2 过期注释 | 已修复 — 改为"已由 §3.4 实现" |
| P2-#4 | HTTP handle 持续累积 | 已知设计取舍，文档已记录 |
| P2-#8 | `restore_into` 不清除 handles | 已知设计取舍，文档注释已说明 |
| **P3-#9** | `resume_multiple_cycles` 测试 hack | **未修复**（见下方 P0-#2） |

---

### P0 — 文档标记完成但代码未实现 / 钻空子

#### P0-#1 `TimerSleep` 仍是 no-op，但 Phase 3.5 已标记"已完成"

**文件：**[`src/vm.rs:771-773`](src/vm.rs:771)

```rust
OpCode::TimerSleep { ms: _ } => {
    // Placeholder: sleep is a no-op in this phase
}
```

PHASE3.md §3.2 的指令表格将 `TimerSleep` 标为占位符，计划在 **3.5 重连阶段**补全。§3.5 的 checklist 虽然不包含 TimerSleep 的具体条目，但该节状态标记为"已完成"——而 §3.2 所声明的依赖并未兑现。

`IoHandle::Timer` 变体存在于 [`src/io.rs`](src/io.rs) 中，但从未被创建或使用。唯一的相关测试是 `timer_sleep_is_noop`，它验证的就是"不 sleep"这一行为。

**影响**：任何依赖 `TimerSleep` 的程序在 snapshot 恢复后计时器行为完全丢失。重连阶段应能重新启动计时器（保存剩余睡眠时间、计算已流逝时间等），但当前没有任何计时信息被保存或恢复。

**建议**：要么在 PHASE3.md §3.5 的状态说明中明确标注"TimerSleep 例外"，要么在下一个迭代中补全计时器支持（创建 `IoHandle::Timer`、记录 `start_instant` 和 `remaining_ms`、在 resume 时重新计算剩余时间）。

---

#### P0-#2 `resume_multiple_cycles` 测试绕过正式 `resume()` API

**文件：**[`src/vm.rs:1877-1878`](src/vm.rs:1877)

```rust
// Cycle 2: manually resume by re-enabling running
vm.running = true;
vm.step().unwrap(); // push 3
```

REVIEW-PHASE3.md P3-#9 已指出此问题，建议改为每个周期用 `vm.resume(&snap)`。当前代码未修改，注释改为"已知 API 限制"。这跳过了正式 resume 流程中的 `reconnect_report` 和 `restore_task_tree`，使多周期 yield-resume 测试无法覆盖真实的 resume 路径。

**影响**：如果 resume 链中有跨周期的 I/O 句柄重连逻辑变化，该测试不会发现回归。

**建议**：将测试改为每个周期独立创建 snapshot 并调用 `vm.resume(&snap)`。如果当前 API 不支持这种方式（`resume` 需要 `running == false`），可考虑在 VM 上增加 `resume_continue` 方法或在测试中显式重建 VM 并逐周期恢复。

---

### P1 — 文档与实现不一致 / 潜在正确性问题

#### P1-#1 父任务 Yield 时不调用 `save_current_task()`

**文件：**[`src/vm.rs:627-670`](src/vm.rs:627)

Yield 分支只设置 `task.status = TaskStatus::Yielded(pc)` 并调用 `Snapshot::capture()`，不调用 `save_current_task()`。

对比：`WaitChildren` handler（line 862）和子任务 Return handler（line 813）都调用了 `save_current_task()` 以持久化状态到 task registry。

当前依赖 `Snapshot::capture()` 直接从 `vm.handles` 读取全局 I/O 段来补救。结果是 handle 归属不对称：

- 子任务的 handles → 序列化到 snapshot 的 `task_section`（通过 `serialize_task_snapshot`）
- 父任务的 handles → 序列化到 snapshot 的全局 `io_section`（通过 `vm.handles`）

resume 后：
- `restore_io_handles` 恢复全局 handles 到 `vm.handles`
- `restore_task_tree` 恢复子任务 handles 到各 task 的 `task_registry[child].io_handles`

当前所有 Phase 3/4 测试碰巧没有"父任务拥有自己的 I/O handles 且同时有子任务"的场景——父任务总是只 spawn + wait + yield。若父任务在 spawn 之前做了 `FileOpen`，对称性破坏可能在未来引入 bug：例如后续代码试图从 `task_registry[parent].io_handles` 读取父任务的 handles 时会发现为空。

**建议**：在 Yield 分支中调用 `save_current_task()` 以保证一致性。由于 `Snapshot::capture()` 同时从 `vm.handles` 和 `task_registry` 读取，需要确保 handle 不会在全局 io_section 和 task_section 中被重复序列化。可以考虑在 capture 中跳过当前任务的 handles（因为它们已在 task_section 中覆盖），或将 io_section 仅用于无任务的兼容路径。

---

#### P1-#2 PHASE4 §4.3 "隐式等待"措辞与代码行为不符

**文件：**[`docs/PHASE4.md`](docs/PHASE4.md) §4.3

文档写：
> v1 规则：Yield 前隐式等待：`OpCode::Yield` 执行前，检查当前任务是否有 `Completed` 状态之外的子任务，若有则报错

"隐式等待"暗示父任务会自动阻塞等待子任务完成。但代码实现（[`src/vm.rs:640-649`](src/vm.rs:640)）是直接返回 `VmError::UncompletedChildren`，而非阻塞：

```rust
if !uncompleted.is_empty() {
    return Err(VmError::UncompletedChildren(uncompleted));
}
```

这一行为与 DESIGN.md 的原始意图一致（"父任务 yield 时，所有子任务必须先完成"），但 PHASE4.md 的"隐式等待"措辞容易造成误解——读者可能认为 VM 会自动等待子任务而不是报错。

**建议**：将"隐式等待"改为"显式检查"或"禁止带未完成子任务"，与代码行为保持一致。

---

### P2 — 设计取舍 / 已知技术债

#### P2-#1 HTTP handle 持续累积

**文件：**[`src/vm.rs`](src/vm.rs) `http_get` / `http_post`

每次 HTTP 调用通过 `self.create_handle()` 创建新 `HttpConnection` handle，永不关闭。已在 PHASE3.md 中记录为已知设计取舍，属于 Phase 5 待解决的技术债。不算钻空子。

---

#### P2-#2 `restore_into` 不处理 I/O handles

**文件：**[`src/snapshot.rs:1140-1143`](src/snapshot.rs:1140)

`restore_into` 不碰 handles，必须配合 `restore_io_handles` 使用。文档注释已说明此耦合。正常路径（`vm.resume()`）会依次调用两者，不会出问题。不建议在当前阶段改动。

---

#### P2-#3 `Snapshot::gc_to_pos` 字段是真正的死代码

**文件：**[`src/snapshot.rs:124-126`](src/snapshot.rs:124)

```rust
#[allow(dead_code)]
gc_to_pos: HashMap<usize, u32>,
```

确认分析：

- `capture()` 中构建的局部变量 `gc_to_pos` 赋值进了 struct 字段
- 但该字段本身从未被任何代码读取——所有序列化 helper（`write_value_ref`、`serialize_frame`、`serialize_heap_object`、`serialize_task_snapshot`）使用的是 `capture()` 中传入的局部变量引用，而非 struct 字段
- 反序列化路径中 `gc_to_pos: HashMap::new()`（line 474）直接丢弃了映射

`#[allow(dead_code)]` 注解是**必要且正确**的。REVIEW-PHASE3.md P3-#6 建议删除此注解的分析是错误的——删除会导致 clippy pedantic 报 `dead_code` 警告。

**建议**：如果希望彻底清理，可以考虑将 `gc_to_pos` 从 struct 中移除（因为它仅在序列化时使用，不需要保存在 snapshot 中）。但这会增加 `capture()` helper 函数参数传递的复杂度。保持现状是合理的。

---

### P3 — 文档质量问题

#### P3-#1 PHASE3.md §3.5 "已完成"标记未附带 TimerSleep 例外说明

**文件：**[`docs/PHASE3.md`](docs/PHASE3.md) §3.5

§3.2 的指令表格将 TimerSleep 的补全目标设为 3.5，§3.5 的 checklist 不包含 TimerSleep 但状态标记为"已完成"。应在 §3.5 的状态说明中注明"TimerSleep 仍为占位符，计时器支持推迟至后续阶段"。

---

#### P3-#2 PHASE3.md §3.5 ResumeBlock 预留 —— 声称预留但未实现

**文件：**[`docs/PHASE3.md`](docs/PHASE3.md) §3.5

文档写：
> `OpCode::Yield` 后跟 `ResumeBlock`（`ok` / `partial` / `error` 三个分支地址），当前阶段可预留操作数位置

实际 `OpCode::Yield` 为零操作数指令（[`src/opcode.rs`](src/opcode.rs)），没有任何预留位置。此项明确推迟至 Phase 5（编译器阶段），不影响当前功能，但文档声称的"已预留"不准确。

**建议**：将措辞改为"计划在 Phase 5 中扩展 Yield 操作数以携带三个分支地址"。

---

### 总结

| 等级 | 编号 | 问题 | 类别 |
|------|------|------|------|
| **P0** | #1 | `TimerSleep` no-op，3.5 标记完成但未补全 | 标记完成/未实现 |
| **P0** | #2 | `resume_multiple_cycles` 绕过正式 API | 钻空子/测试 hack |
| **P1** | #1 | 父任务 Yield 不调用 `save_current_task()` | handle 归属不对称 |
| **P1** | #2 | PHASE4 §4.3 "隐式等待"描述与代码行为不符 | 文档/实现不一致 |
| **P2** | #1 | HTTP handle 持续累积 | 已知设计取舍（已记录） |
| **P2** | #2 | `restore_into` 不处理 I/O handles | 已知设计取舍（已记录） |
| **P2** | #3 | `gc_to_pos` struct 字段是死代码 | 已知技术债（注解正确） |
| P3 | #1 | §3.5 "已完成"标记缺少 TimerSleep 例外说明 | 文档质量 |
| P3 | #2 | §3.5 ResumeBlock 预留声称不准确 | 文档质量 |

**整体评价**：核心的 snapshot/resume/task 机制实现扎实，REVIEW-PHASE3 中发现的 P0/P1 代码问题（Append 模式、value_to_bytes、错误处理等）已正确修复。最值得关注的是 **TimerSleep 的缺失**——它直接影响依赖计时器的 snapshot-resume 场景的正确性。`resume_multiple_cycles` 的测试 hack 虽然在当前覆盖范围内无害，但会持续积累测试债务。父任务 handle 归属不对称问题在当前测试覆盖下不暴露，但增加了未来维护风险。

---

## 解决方案计划

### P0-#1 `TimerSleep` no-op → 推迟至 Phase 5

**决策**：不在 Phase 4 补全。理由：
1. Timer 的恢复模型（剩余时间重算）不属于 Replay/Seek/Cached 三元组，本质上是第四种策略
2. `sleep` 最终是语言关键字，恢复语义需和 `yield resume { ... }` 语法一起设计，属于 Phase 5 编译器阶段
3. 当前 no-op 不影响 Phase 4 的任何并发测试（没有测试依赖真实时间流逝）

**记录**：已在 PHASE3.md §3.5 状态说明中添加"已知例外"标注，PHASE5.md 路线图将包含完整的 Timer 实现条目。

**影响范围**：仅文档。`IoHandle::Timer` 变体保留不动，Phase 5 直接使用。

---

### P0-#2 `resume_multiple_cycles` 测试绕过正式 API

**计划方案**：将测试拆分为两个独立场景：
1. **yield → snapshot → resume → 验证**（正常单周期，已覆盖）
2. **yield → snapshot → 新建 VM → resume → 再 yield → snapshot → 新建 VM → resume → 验证**（多周期，每周期走完整 `vm.resume()` 路径）

这需要测试中显式创建新 VM 并逐周期恢复，避免操作 `vm.running = true` 内部字段。

**替代方案**：在 VM 上增加 `pub fn resume_continue(&mut self)` 方法，封装"恢复 running 状态"逻辑，使测试不触碰内部字段。

**推荐**：重建 VM 方案（更干净，不引入新 API，且更接近真实使用场景）。

---

### P1-#1 父任务 Yield 时不调用 `save_current_task()`

**计划方案**：
1. 在 `OpCode::Yield` 分支中，`task.status = TaskStatus::Yielded(pc)` 之后、`Snapshot::capture()` 之前，调用 `self.save_current_task()`
2. 修改 `Snapshot::capture()`：当从 `vm.handles` 收集全局 I/O handles 时，跳过当前任务所属的 handles（因为它们已在 `serialize_task_snapshot` 中通过 `task_registry` 覆盖）
3. 在 `capture()` 的 `serialize_task_snapshot` 中增加去重逻辑：如果某 handle 已在 task_section 中序列化，则不在 io_section 中重复

**验证**：新增测试场景"父任务在 spawn 前执行 FileOpen，yield-resume 后父任务 handles 正确恢复"，覆盖当前缺失的对称性路径。

---

### P1-#2 PHASE4 §4.3 "隐式等待"措辞

**计划方案**：将 PHASE4.md §4.3 中的：
```
v1 规则：Yield 前隐式等待：OpCode::Yield 执行前，检查当前任务是否有 Completed 状态之外的子任务，若有则报错
```
改为：
```
v1 规则：Yield 前显式检查：OpCode::Yield 执行前，检查当前任务是否有 Completed 状态之外的子任务，若有则返回 VmError::UncompletedChildren
```

**影响范围**：仅文档措辞，不涉及代码改动。

---

### P2-#1 HTTP handle 持续累积

**状态**：已知设计取舍，已在 PHASE3.md 末尾记录。推迟至 Phase 5 设计合理的 handle 生命周期策略（TTL、引用计数或显式 close 指令）。

---

### P2-#2 `restore_into` 不处理 I/O handles

**状态**：已知设计取舍，文档注释已说明调用方必须配合 `restore_io_handles`。正常路径（`vm.resume()`）依次调用两者，不会出错。暂不修改。

---

### P2-#3 `gc_to_pos` struct 字段是死代码

**状态**：`#[allow(dead_code)]` 注解必要且正确。保持现状。

---

### P3-#1 §3.5 "已完成"缺少 TimerSleep 例外说明

**已修复**：PHASE3.md §3.5 状态说明下方已追加"已知例外"标注。

---

### P3-#2 §3.5 ResumeBlock 预留声称不准确

**已修复**：PHASE3.md §3.5 中 `ResumeBlock` 条目措辞已改为"计划在 Phase 5 中扩展为携带..."。
