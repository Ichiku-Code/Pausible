## Phase 4 路线图：结构化并发

**目标**：多任务程序的挂起与恢复。引入 `spawn` 创建子任务，任务之间形成严格的树状关系，
父任务必须等待所有子任务完成后才能 yield。Snapshot 捕获完整任务树，Resume 时重建。

### 4.1 任务模型与数据结构
> **状态：已完成。**

- `TaskId` 类型：唯一标识一个任务（`usize` 包装）
- `Task` 结构体：`id`, `parent: Option<TaskId>`, `children: Vec<TaskId>`, `status: TaskStatus`, `stack: Vec<Value>`, `frames: Vec<CallFrame>`, `io_handles: HashMap<HandleId, IoHandle>`
- `TaskStatus` 枚举：`Running` / `Yielded(pc: usize)` / `Completed`
- `TaskTree` 数据结构：管理所有任务的父子关系，提供 `add_child` / `get_children` / `find_parent` 等查询
- VM 集成：`task_registry: HashMap<TaskId, Task>`，`current_task_id: TaskId`，调度器负责切换活跃任务
- 子任务继承父任务的函数表（`functions: Arc<Vec<Function>>` 共享引用）

### 4.2 Spawn 指令
> **状态：已完成。**

- `OpCode::Spawn(func_id)`：创建子任务，以指定函数为入口点
- 子任务获得独立的操作数栈和调用帧，初始 `locals` 为空
- 父任务继续执行（不阻塞），子任务进入 `Running` 状态等待调度
- 任务树更新：父任务的 `children` 列表追加新 `TaskId`
- 子任务的 `parent` 字段指向父任务 `TaskId`

### 4.3 任务完成与等待
> **状态：已完成。**

- `OpCode::WaitChildren`：父任务阻塞，直到所有直接子任务完成（`TaskStatus::Completed`）
- 子任务完成时将其栈顶值作为返回值传递给父任务
- 返回值收集方式：子任务执行完毕后 `push_return_value`，父任务 `WaitChildren` 后从 `child_returns: Vec<Value>` 获取
- `TaskStatus` 转换：`Running` → `Completed`（子任务执行到函数末尾或 `Return` 指令）
- **v1 规则：Yield 前隐式等待**：`OpCode::Yield` 执行前，检查当前任务是否有 `Completed` 状态之外的子任务，若有则报错

### 4.4 Yield 与任务树
> **状态：已完成。**

- 扩展 `Snapshot::capture`：遍历整个任务树，而不是单个任务的栈帧
- 只捕获 `Completed` 和 `Yielded(pc)` 状态的任务（无 `Running` 状态任务）
- `TaskSnapshot` 结构：`id`, `parent`, `status`, `stack`, `frames`, `io_handles`
- `Snapshot` 新增 `task_tree: Vec<TaskSnapshot>` 字段
- `SnapshotHeader` 新增 `task_count: u32` 字段

### 4.5 Resume 与任务树重建

> **状态：已完成。**

- 从 `task_tree` 反序列化所有 `TaskSnapshot`，重建 `Task` 对象
- 恢复父子关系（先恢复所有任务 ID，再建立 parent/children 链接）
- 找到 `Yielded(pc)` 状态的任务，将其设置为 `current_task_id`
- 重连阶段按任务维度处理 I/O 句柄（每个任务独立 rebuild 自己的 handles）

### 4.6 并发 I/O
> **状态：已完成。**


- 多个子任务各自执行 I/O 操作（HTTP GET、File Read 等），I/O 句柄归属创建任务的 registry
- 任务级句柄隔离：每个 `Task` 有自己的 `io_handles: HashMap<HandleId, IoHandle>`
- 父任务 `WaitChildren` 时所有子任务已完成，其 I/O handles 已关闭或在 snapshot 时被捕获
- Snapshot 捕获时遍历所有任务的 `io_handles`，按已有机型序列化
- Resume 重连时逐任务重建 I/O 句柄，汇总为全局 `ReconnectReport`

### 4.7 测试与验证

#### 4.7.1 基础并发测试

- [x] **spawn 单子任务 → wait → 获取返回值**：父任务 spawn 一个计算斐波那契的子任务，`WaitChildren` 后获取正确结果
- [x] **spawn 多子任务 → wait → 全部完成**：父任务 spawn 3 个子任务各自计算阶乘，`WaitChildren` 后验证所有返回值
- [x] **Yield 前子任务未完成 → 报错**：父任务 spawn 子任务后直接 yield（不 wait），验证 VM 返回错误
- [x] **嵌套 spawn**：父任务 spawn 子任务 A，A spawn 孙任务 B，验证三层任务树正确性

#### 4.7.2 任务树 Snapshot 与恢复

- [x] **单子任务 yield-resume 周期**：父任务 spawn 子任务 → wait → yield → snapshot → resume，验证子任务返回值在 resume 后仍可访问
- [x] **多子任务全部完成后 yield-resume**：父任务 spawn 3 个子任务 → wait 全部 → yield → snapshot → resume，验证任务树完整恢复
- [x] **v3 含任务树 snapshot 文件往返**：含多任务树的 snapshot 写文件后再读回，数据一致

#### 4.7.3 并发 I/O 测试

- [x] **并发 HTTP → wait → yield → resume**：父任务 spawn 2 个子任务各自发 HTTP GET → wait → yield → snapshot → resume，验证 Replay 策略正常重放
- [x] **并发文件读取 → wait → yield → resume**：子任务 FileOpen + FileRead → wait → yield → snapshot → resume，验证子任务 I/O 句柄保留且可恢复
- [x] **混合 I/O 类型并发**：子任务 A 做 File Read，子任务 B 做 HTTP GET，子任务 C 用 Stdin，yield → resume 后验证三种策略均正确恢复

#### 4.7.4 错误路径测试

- [x] **spawn 无效 func_id → 错误**：Spawn 引用不存在的函数 ID，验证 VM 返回 `VmError::UndefinedFunction`
- [ ] **子任务执行失败不阻塞父任务 wait**：子任务运行时遇到除零错误，父任务 `WaitChildren` 应收到错误信号并传播
- [ ] **任务树深度限制**：嵌套 spawn 超过合理深度（如 256）时返回错误，防止栈溢出

**依赖顺序**：4.1 -> 4.2 -> 4.3 -> 4.4 -> 4.5 -> 4.6 -> 4.7。
