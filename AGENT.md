# Pausible — AGENT.md

可暂停/恢复的字节码虚拟机，Rust edition 2024。核心流程：执行计算任务 → Yield 挂起 → Snapshot 序列化 → Resume 恢复。

项目结构：`src/` 下按模块拆分为 value、heap、vm、opcode、function、chunk，通过 `lib.rs` 公开。`docs/DESIGN.md` 是整体架构设计，`docs/PHASE2.md` 是当前阶段路线图。具体模块职责和 API 从源码获取，此文件不再维护冗余的逐文件清单。

## 架构模式

### Value 设计原则

高频内置类型（`String`、`List`）作为 `Value` 的直接变体，保证访问性能和模式匹配简洁。低频和未来可能的用户自定义类型统一走单一变体，避免变体爆炸。详见 `docs/DESIGN.md`。

### Gc<T> 与堆

- `Gc<T>` 是 `Copy` 索引句柄，不负责释放。
- `Heap` 使用 `Vec<HeapObject>` + `free_slots: Vec<usize>` 的自由链表 arena。
- 类型投影通过 `HeapAccess` trait，新增堆类型只需实现该 trait。

### 标记-清除 GC

- **索引稳定性**：GC 从不删除元素，死槽回收进 `free_slots` 复用。`Gc<T>` 句柄在任意次 GC 后始终有效。
- **根扫描**：遍历操作数栈 + 所有 `CallFrame.locals`，与 Snapshot 序列化复用同一套根遍历机制。
- **递归标记**：先收集子索引到临时 `Vec<usize>` 再递归，避免 `self.objects` 与 `self.marked` 的借用冲突。
- **触发策略**：每次分配后 `maybe_gc()`，live 对象数超过阈值（默认 256）时自动触发。
- **API 边界**：GC 内部方法（`mark`/`mark_value`/`reset_marks`）均为 `pub(crate)`，外部只能通过 `VM::collect_garbage()` 触发。

## Clippy 配置

`Cargo.toml` 中 `[lints.clippy]` 设置 pedantic 级别为 `warn`。提交前必须零警告。运行方式：

```bash
cargo clippy --tests -- -W clippy::pedantic
```

## 常见陷阱

### apply_patch 与中文

`apply_patch` 处理含非 ASCII 字符的文件时，上下文匹配经常失败。中英混排文件的编辑优先用 `sed` 或 `python3 -c` 做精确行替换。

### Git 操作需要沙箱提权

`.git` 在沙箱中是只读的，`git commit`/`git add` 需要 `sandbox_permissions: "require_escalated"`。

### 借用冲突（GC 递归标记）

GC 标记阶段不能直接对 `self.objects` 和 `self.marked` 做嵌套借用。正确做法：先用不可变借用收集子索引到临时 `Vec`，释放借用后再递归。

### 未使用变量

`#[cfg(test)]` 中的未使用变量也会产生 warning，加 `_` 前缀即可（如 `_gc`）。

### clippy::approx_constant

测试中避免使用 `3.14`、`2.718` 等近似常量的浮点字面量，用 `1.5`、`2.0` 等不会误匹配的值。

## OpCode::Call 参数顺序

调用方从左到右 push 参数，`Call` 指令从栈顶弹出（逆序），然后反转存入 `locals`。因此 `locals[0]` 对应第一个参数，`locals[arity-1]` 对应最后一个。

## 提交约定

格式：`类型: 中文描述`。类型为 `feat:`/`docs:`/`fix:`。

## 测试模式

VM 测试常用两个辅助函数：

```rust
fn make_vm(code: Vec<OpCode>) -> VM                     // 创建含单函数的 VM 并 prepare
fn run_code(code: Vec<OpCode>) -> Result<VM, VmError>  // make_vm + run
```

GC 测试不依赖阈值触发，手动调用 `vm.collect_garbage()` 保证确定性：

1. 分配对象，push 到栈使其可达
2. 调用 `vm.collect_garbage()` 验证可达对象存活
3. `vm.stack.pop()` 后再次 GC，验证不可达对象被回收
4. 通过 `vm.heap.len()` 检查 live 对象数，`vm.heap.capacity()` 确认槽位未删除
