## Phase 2 路线图：Snapshot

**目标**：纯计算程序（含堆分配对象）的保存与恢复。

### 2.1 引用类型与堆

- `String` 类型：堆分配的 UTF-8 字节序列 ✅
- `List<T>` 类型：堆分配的动态数组 ✅
- `Gc<T>` 智能指针包装，统一管理堆对象生命周期 ✅
- 堆管理器：分配、追踪所有堆对象 ✅

### 2.2 标记-清除 GC

- 根集扫描：遍历所有 CallFrame 的 locals 和操作数栈 ✅
- 标记阶段：从根出发递归标记所有可达堆对象 ✅
- 清除阶段：回收未标记对象，释放堆内存 ✅
- 触发策略：堆达到阈值时自动触发 ✅
- 索引稳定性：GC 不删除 Vec 元素，死槽回收至 free_slots ✅
### 2.3 序列化基础设施

- `Serializable` trait：`serialize(&self, ctx: &mut SerCtx)` / `deserialize(ctx: &mut DeCtx) -> Self`
- `SerCtx` / `DeCtx`：序列化/反序列化上下文，管理对象引用映射
- 所有 `Value` 类型（含 `String`、`List`）实现 `Serializable`

### 2.4 Snapshot 格式

- 文件头：magic `"PAUS"`、version、code_hash、timestamp
- 堆对象序列化：复用 GC 根扫描定位可达对象，按 DFS 顺序写入
- 栈帧序列化：所有 `CallFrame` 的 `locals` 和 `ip`
- 全局值表：命名全局变量与值的映射
- 文件读写：`Snapshot::write(path)` / `Snapshot::read(path)`

### 2.5 Yield 指令

- `OpCode::Yield`：挂起当前程序
- VM 执行流程：GC 扫描 → 序列化快照 → 写入文件 → 终止运行
- 无 I/O、无并发：yield 点只能出现在纯计算任务中

### 2.6 恢复 (Resume)

- 从 snapshot 文件加载 VM 状态
- 重建堆：反序列化所有堆对象
- 重建调用栈：恢复所有 `CallFrame`，`ip` 指向 yield 的下一条指令
- 重建函数表：用 `code_hash` 校验字节码版本一致性

### 2.7 跨架构验证与测试

- [ ] 斐波那契 yield 中途保存，恢复后得出正确结果
- [ ] 阶乘嵌套 yield（多次保存/恢复）
- [ ] 带循环的程序：迭代中 yield，resume 后继续
- [ ] 端序一致性验证（`to_le_bytes` / `from_le_bytes`）
- [ ] 浮点数跨平台确定性验证（IEEE 754 原始字节存储）

**依赖顺序**：2.1 -> 2.2 -> 2.3 -> 2.4 -> 2.5 -> 2.6 -> 2.7
