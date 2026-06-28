## 代码审查报告 — Phase 3

日期：2026-06-28  
核实日期：2026-06-28

### 测试状态

- 单元测试 + 集成测试：177 passed, 0 failed
- Clippy pedantic：零警告

---

### P0 — 数据正确性问题

#### 1. `write_file_handle` 忽略 `FileMode::Append`，始终截断文件

**文件：** [`src/vm.rs:687-735`](src/vm.rs:687)

**核实结果：真实。** 所有写路径硬编码 `.truncate(true)` 打开文件，`mode` 字段完全未被读取。当前无论 handle 的 mode 是 Read、Write 还是 Append，写入时一律截断文件从头写。以 Append 模式创建的文件在首次写入时会被清空，等同于 Write 模式。

连带问题：所有文件 I/O 错误均映射为 `VmError::HeapError(HeapError::InvalidHandle)`，对调试无帮助。

**计划修复：** 在写路径根据 `mode` 选择 `OpenOptions`：
- `FileMode::Append` → `.append(true).create(true)`（不 truncate）
- `FileMode::Write` 或未指定 → `.write(true).create(true).truncate(true)`
- `FileMode::Read` 上尝试写 → 返回错误
- 所有 I/O 错误改用 `VmError::IoError(...)` 报告

---

#### 5. `value_to_bytes` 对 String 和 List 返回空 `Vec<u8>`

**文件：** [`src/vm.rs:926-948`](src/vm.rs:926)

**核实结果：真实。** `Value::String(gc)` 和 `Value::List(gc)` 分支返回空 `Vec`。注释称"由 VM 层检查"，但调用方（`write_file_handle`、`write_stdout` 等）均未做类型校验。将 String 值写入文件或 stdout 会静默输出零字节。

同一文件中的 `value_to_string` 函数已正确解析 GC 引用。

**计划修复：** `value_to_bytes` 接受 `&Heap` 参数，对 String 值通过 heap 解析 GC 引用返回实际字节。List 保持返回空（List 写入 I/O 通道的语义留待后续定义）。

---

### P1 — 错误处理 / 行为不一致

#### 2. `write_file_handle` 对非 File handle 静默返回 `Ok(0)`

**文件：** [`src/vm.rs:687-735`](src/vm.rs:687)

**核实结果：真实。** 入口 match 的 fallthrough `_ => return Ok(0)` 使得将 TcpStream 或 HttpConnection handle 传入 FileWrite 时静默返回零字节写入成功。对比 `tcp_read_handle`（src/vm.rs:790）对非 TcpStream handle 正确地返回 `Err(VmError::IoError(...))`。

**计划修复：** fallthrough 分支返回 `Err(VmError::IoError("handle is not a File".into()))`，与 `tcp_read_handle` 的行为保持一致。

---

#### 3. `write_stdout` / `write_stderr` 无 handle 时静默丢弃数据

**文件：** [`src/vm.rs:764-786`](src/vm.rs:764)

**核实结果：真实。** 函数返回 `()`，无法区分"写入了 buffer"和"没有 handle 所以丢弃了"。同时 `StdinRead` 在无 Stdin handle 时返回 `Value::Null`，三个标准流行为不一致。

**计划修复：** 将 `write_stdout` / `write_stderr` 返回值改为 `Result<usize, VmError>`（与 FileWrite 一致）。无 handle 时返回 `Err(VmError::IoError("no Stdout handle".into()))`。`StdinRead` 保持返回 `Value::Null` 的行为不变（读空的语义合理），在文档中注明差异。

---

### P2 — 设计问题

#### 4. `http_get` / `http_post` 每次调用都创建新 handle

**文件：** [`src/vm.rs:844-919`](src/vm.rs:844)

**核实结果：真实。** 每次 HTTP 调用通过 `self.create_handle()` 创建新 `HttpConnection` handle，永不关闭，持续累积在 registry 中。但这不完全是无意泄漏——在当前 snapshot-replay 设计中，保留 HTTP 调用记录是为了 snapshot 能捕获所有 I/O 操作历史。

**计划修复：** 当前阶段不做改变。在 Phase 4（资源管理）或 Phase 5（编译器集成）中设计合理的 handle 生命周期策略（如基于 TTL、引用计数或显式 close 指令）。在 PHASE3.md 中记录为已知设计取舍。

---

#### 8. `restore_into` 不清除 `vm.handles`

**文件：** [`src/snapshot.rs:988-1040`](src/snapshot.rs:988)

**核实结果：半真半假。** `restore_into` 确实不碰 handles，但正常路径（`vm.resume()`）会接着调用 `restore_io_handles`，后者通过 `HashMap::insert` 用相同 HandleId 覆盖旧值。实际运行时不会出现数据损坏，因为 snapshot 中的 HandleId 与 VM 中已有的一致。真正的问题是 API 层面的隐式耦合：如果单独调用 `restore_into` 而不调用 `restore_io_handles`，旧 handle 会残留。

**计划修复：** 在 `restore_into` 的文档注释中明确说明它不处理 I/O handles，必须配合 `restore_io_handles` 使用。当前阶段不做代码级更改。

---

### P3 — 风格 / 文档 / 测试质量

#### 6. Snapshot 结构体重复 `#[allow(dead_code)]`

**文件：** [`src/snapshot.rs:125-126`](src/snapshot.rs:125)

**核实结果：真实。** 第 125-126 行两个相邻的 `#[allow(dead_code)]`。`gc_to_pos` 在 `capture()` 中赋值使用，并非死代码，两个注解均可移除。

**计划修复：** 删除两行 `#[allow(dead_code)]`。

---

#### 7. PHASE3.md §3.2 注释过期

**文件：** [`docs/PHASE3.md`](docs/PHASE3.md)

**核实结果：真实。** §3.2 表格后存在：

> 还未实现。当前 Yield 只设置 running = false，不触发句柄快照。此逻辑在 3.4 中实现。

3.4 已完成，Yield 已触发 snapshot 捕获。该注释与 §3.4（标记完成）矛盾。

**计划修复：** 删除过期注释，或改为"Yield 时触发 snapshot 捕获 — 已由 §3.4 实现"。

---

#### 9. 测试 hack：`resume_multiple_cycles` 手动设置 `vm.running = true`

**文件：** [`src/vm.rs:1554-1581`](src/vm.rs:1554)

**核实结果：真实。** 第 1581 行 `vm.running = true` 绕过正式的 `resume()` 流程。测试想做多周期 yield-resume，但第二次周期没有对应的 snapshot 可传入 `resume()`。这个 hack 暴露了 API 缺少"不带 snapshot 继续运行"的能力，但并未掩盖真实 resume 路径的 bug（那些已有独立测试覆盖）。

**计划修复：** 将测试改为每个周期用 `vm.resume(&snap)` 的方式：周期 1 创建 snapshot 并 resume → 周期 2 再创建 snapshot 并 resume。如果 API 不支持这种方式（当前 `resume` 需要 `running == false`），则考虑在 VM 上增加 `continue_after_resume` 或者在测试注解中说明 hack 的目的。

---

### 备注（非问题，无需修复）

- `TimerSleep` 是有文档记录的占位符 no-op（PHASE3.md 已注明）
- `FileOpen` 创建 handle 时不打开文件（延迟到首次读写时打开），是合理的延迟模式
- `read_u32_or` 在短读取时静默返回默认值，在 `reconnect_handle` 中用于可选字段解析，但会掩盖 params 缓冲区损坏——当前可接受，因为 snapshot 数据由同进程写入，损坏概率极低
