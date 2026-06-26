## 项目概览

Pausible 是一个可暂停/恢复的字节码虚拟机，用 Rust (edition 2024) 实现。核心能力：执行计算任务，支持 Yield/Snapshot/Resume 流程。

## 模块地图

| 文件 | 职责 |
|------|------|
| `src/value.rs` | `Value` 枚举（Null/Bool/Int/Float/String/List）、类型运算、Display |
| `src/heap.rs` | `Gc<T>` 句柄、`HeapObject` 枚举、`Heap`（自由链表 arena + 标记-清除 GC）、`HeapAccess` trait |
| `src/vm.rs` | `VM`、`CallFrame`、`VmError`、26 条指令的执行循环、GC 根扫描与触发 |
| `src/opcode.rs` | `OpCode` 枚举（26 条指令），含 Push/算术/比较/逻辑/控制流/局部变量/函数调用/Halt/Yield |
| `src/function.rs` | `Function` 模型（名称、arity、num_locals、字节码） |
| `src/chunk.rs` | 字节码构造器 (`ChunkBuilder`) 与二进制序列化/反序列化（Magic `"SKLP"`、版本校验） |
| `src/lib.rs` | crate 根，声明所有公开模块 |
| `docs/DESIGN.md` | 架构设计文档（类型系统、堆/GC、I/O 分类） |
| `docs/PHASE2.md` | Phase 2 路线图（Snapshot 全流程） |

## 架构模式

### Value 设计原则

高频内置类型（`String`、`List`）作为 `Value` 的直接变体，保证访问性能和模式匹配简洁。低频和未来可能的用户自定义类型统一走单一 `UserObj` 变体，避免变体爆炸。详见 `docs/DESIGN.md`。

### Gc<T> 与堆

- `Gc<T>` 是 `Copy` 的索引句柄（`usize` + `PhantomData<T>`），不负责释放。
- `Heap` 是 `Vec<HeapObject>` + `free_slots: Vec<usize>` 的自由链表 arena。
- 类型投影通过 `HeapAccess` trait 实现，新增堆类型只需实现该 trait。

### 标记-清除 GC

- **索引稳定性**：GC 从不删除 `Vec` 元素，死槽回收进 `free_slots` 供后续复用。
- **根扫描**：`VM::mark_roots()` 遍历操作数栈和所有 `CallFrame.locals`。
- **递归标记**：先收集子索引到临时 `Vec<usize>` 再递归，避免 `self.objects` 和 `self.marked` 的借用冲突。
- **触发策略**：每次分配后调用 `maybe_gc()`，当 live 对象数超过阈值（默认 256）时自动触发。
- **API 安全**：`Heap::mark` / `mark_value` / `reset_marks` 等 GC 内部方法标记为 `pub(crate)`，外部只能通过 `VM::collect_garbage()` 触发。

## 开发工作流

```bash
# 编译（快）
cargo build

# 运行全部测试（77 单元 + 5 集成）
cargo test

# Pedantic clippy（含测试代码）
cargo clippy --tests -- -W clippy::pedantic
```

Clippy pedantic 级别在 `Cargo.toml` 的 `[lints.clippy]` 中配置为 `warn`，提交前必须零警告。

## 常见陷阱

### `clippy::approx_constant`

使用 `3.14` 等近似 PI 的浮点字面量会触发此 lint。测试中需要用浮点数时，避免使用 `3.14`、`2.718` 等魔数，用 `1.5`、`2.0` 等不会误匹配常量库的值。

### `clippy::missing_errors_doc`

返回 `Result` 的公开函数需要 `# Errors` 文档段落。如确有大量重复场景，可在模块级用 `#[allow(...)]` 抑制，但应谨慎。

### apply_patch 与中文

`apply_patch` 工具在处理包含中文（非 ASCII）字符的文件时，上下文匹配经常失败。遇到中英混排文件编辑时，优先用 `sed` 或 `python3 -c` 做精确行替换。

### Git 操作需要提权

`.git` 在沙箱中是只读的，`git commit` / `git add` 需要 `sandbox_permissions: "require_escalated"`。前缀规则格式：`["git", "-C", "/home/zz_404/rust/pausible", "commit", "-m", "..."]`。

### 借用冲突与递归标记

在 GC 标记阶段，不能直接对 `self.objects` 和 `self.marked` 做嵌套借用。正确做法是先用不可变借用收集子索引到临时 `Vec`，释放借用后再递归标记。

### 未使用变量在测试代码中

Rust 编译器对 `#[cfg(test)]` 中的未使用变量也会产生 warning。已声明但不使用的变量应加 `_` 前缀（如 `_gc`、`_b`）。

## 提交约定

提交信息格式遵循 `类型: 中文描述`：

- `feat:` — 新功能实现
- `docs:` — 文档变更
- `fix:` — 修复

示例：
```
feat: 实现 2.2 标记-清除 GC (自由链表 + 根扫描 + 阈值触发)
docs: 补充自由链表 GC 索引稳定性设计
```

## 测试模式

### VM 测试辅助函数

```rust
fn make_vm(code: Vec<OpCode>) -> VM  // 创建含单函数的 VM 并 prepare
fn run_code(code: Vec<OpCode>) -> Result<VM, VmError>  // make_vm + run
```

### GC 测试模式

GC 测试不依赖 `maybe_gc()` 的阈值触发，而是手动调用 `vm.collect_garbage()` 确保确定性：

1. 分配对象，通过 `vm.stack.push(Value::String(gc))` 使其可达
2. 调用 `vm.collect_garbage()` 验证可达对象存活
3. `vm.stack.pop()` 后再次 GC，验证不可达对象被回收
4. 通过 `vm.heap.len()` 检查 live 对象数，通过 `vm.heap.capacity()` 确认槽位未被删除

### OpCode::Call 参数顺序

调用方从左到右 push 参数，`Call` 指令从栈顶弹出（逆序），然后反转存入 `locals`。因此 `locals[0]` 对应第一个参数，`locals[arity-1]` 对应最后一个。
