# Pausible 语言设计方案

## 一、总体架构

Pausible 是一个跑在自定义字节码 VM 上的语言，核心命题是：**程序可以在自身选择的时机挂起，保存为可移植的 snapshot，之后在同一台或另一台设备上恢复并继续执行。**

```
┌──────────────────────────────────────┐
│           Pausible 源码               │
├──────────────────────────────────────┤
│  编译器 → 类型检查 → 字节码生成        │
├──────────────────────────────────────┤
│         Pausible VM                   │
│  ┌────────────────────────────────┐  │
│  │  字节码解释器                   │  │
│  │  栈 + 堆管理 + GC               │  │
│  │  结构化并发调度器                │  │
│  │  I/O 管理器（内置接口）          │  │
│  │  Snapshot 序列化 / 反序列化      │  │
│  │  Resume 重连阶段                │  │
│  └────────────────────────────────┘  │
└──────────────────────────────────────┘
```

Snapshot 是架构无关的。它不 dump 原生内存，而是序列化 VM 内部的语义结构——栈帧、堆对象、类型表、I/O 句柄、任务树。

---

## 二、关键设计细节

### 2.1 字节码 VM

**栈式虚拟机**，原因是在做 snapshot 时栈帧天然自包含——每个帧的局部变量就在帧内，不需要跨寄存器追踪。

- 所有值带类型标签（tagged value），不做裸指针
- GC 用追踪式（tracing GC），snapshot 时复用 GC 的根扫描机制来定位所有可达对象
- I/O 操作是 VM 指令，不经过 FFI。VM 拦截每一次 I/O，记录参数和结果

### 2.2 类型系统

强静态类型。所有类型必须实现 `Serializable` trait：

```
trait Serializable {
    fn serialize(&self, ctx: &mut SerCtx)
    fn deserialize(ctx: &mut DeCtx) -> Self
}
```

内置类型：`Int`, `Float`, `Bool`, `String`, `Bytes`, `List<T>`, `Map<K,V>`, `Option<T>`, `Result<T,E>`

I/O 句柄类型：`File`, `TcpStream`, `HttpConnection`, `Timer`

### 2.3 I/O 系统——分类与重建

这是整个设计中最需要精细化的部分。Pausible 把 I/O 分为三类，各有不同的恢复策略：

| 类别 | 语义 | 例子 | Snapshot 中保存 | Resume 时的行为 |
|---|---|---|---|---|
| **Replayable** | 可重放 | HTTP GET, DB 查询 | 请求参数 + 上次响应 | 重新发起请求；若结果不同，触发 `DataDiverged` 事件 |
| **Seekable** | 可定位 | 文件读写 | 路径 + 偏移量 | 重新打开 + seek；若文件不存在或变短，触发 `ResourceLost` |
| **Ephemeral** | 一次性 | stdin, 传感器 | 完整缓存的数据 | 直接使用 snapshot 中的缓存值 |

每种 I/O 句柄在创建时由程序员显式标注其类别（或由编译器推导），这决定了它被 snapshot 捕获的方式和 resume 时的重建逻辑。

### 2.4 结构化并发与 yield 语义

任务之间是严格的树状关系。`spawn` 创建子任务，父任务不能在自己完成前让子任务悬空。

**v1 规则：父任务 yield 时，所有子任务必须先完成。**

```
task parent {
    spawn { fetch(url_a) }          // 子任务 A
    spawn { fetch(url_b) }          // 子任务 B
    // 隐式等待 A、B 都完成
    yield                           // 此时子任务已结束，snapshot 只存父任务状态
}
```

这个规则把"暂停子任务中间状态"的复杂度推迟到后续版本。它保证了 snapshot 中的任务树只包含"已完成"或"单点 yield"两种状态，没有"卡在 I/O 半路的子任务"。

等这个跑稳了，v2 可以放宽：子任务在下一个 yield 点暂停，与父任务一起冻结。

### 2.5 自挂起（yield）的语义

`yield` 是一个语言关键字，类似 `await` 的语法位置：

```
fn process() -> Result<(), Error> {
    let data = http.get("https://api.example.com/data")?   // I/O，结果被记录
    let result = transform(data)
    yield                                                    // 此处挂起
    db.write("results", result)?                             // I/O，resume 后重建连接
}
```

`yield` 的执行流程：

1. 当前任务的所有子任务完成
2. VM 扫描 GC 根集（栈帧 + 全局变量 + I/O 句柄），序列化所有可达对象
3. 输出 snapshot 到指定位置
4. 程序终止（或进入等待 resume 状态）

Resume 时的执行流程：

1. VM 加载 snapshot，重建堆和栈
2. 进入 **重连阶段**：遍历所有 I/O 句柄，按类别尝试重建
3. 重连结果汇总为 `ReconnectReport`，程序可选择性处理
4. 若所有关键句柄重建成功，PC 跳回 yield 的下一条指令，继续执行

### 2.6 重连阶段设计

重连不是悄悄发生的，而是有显式语义的阶段。程序可以选择处理重连失败：

```
yield resume {
    ok => {
        // 所有句柄重建成功，正常继续
    }
    partial(report) => {
        // 部分句柄重建失败，report 列出了每个句柄的状态
        // 可以 fallback、retry、或者提前终止
    }
}
```

没有写 `resume` 块时，默认行为是：任何一个非 Optional 的 I/O 句柄重建失败，程序以 `ReconnectError` 终止。

### 2.7 Snapshot 格式（逻辑结构）

```
Snapshot {
    header: {
        magic:      [u8; 4],   // "PAUS"
        version:    u32,
        code_hash:  [u8; 32],  // 字节码的哈希，用于版本匹配
        timestamp:  u64,
    }
    type_table:    Vec<TypeDescriptor>,
    global_values: Map<String, Value>,
    task_tree:     Vec<TaskSnapshot>,
    heap:          Vec<HeapObject>,
}

TaskSnapshot {
    id:        TaskId,
    parent:    Option<TaskId>,
    status:    Completed | Yielded(pc)
    stack:     Vec<StackFrame>,
    io_handles: Vec<IoHandleSnapshot>,
}

StackFrame {
    func_id: usize,
    locals:  Vec<Value>,
}

IoHandleSnapshot {
    id:       HandleId,
    kind:     File | Tcp | Http | Timer | Stdin | Stdout,
    params:   Map<String, Value>,    // 重建连接用的参数
    cached:   Option<Bytes>,         // Ephemeral 类的缓存数据
    position: Option<u64>,           // Seekable 类的偏移量
    strategy: Replay | Seek | Cached,
}
```

---

## 三、实现路线图

### Phase 1：最小 VM（约 4–6 周）

**目标**：一个能跑纯计算程序的字节码解释器。

- 栈式字节码指令集定义（约 30–40 条指令）
- 基本类型系统（int, bool, 简单算术 + 比较）
- 函数调用与返回，局部变量
- 简单的标记-清除 GC
- 用 Rust 或 Zig 实现，无外部依赖

**交付物**：能跑斐波那契、阶乘等纯计算程序。

### Phase 2：Snapshot（约 3–4 周）

**目标**：纯计算程序的保存与恢复。

- 堆遍历与序列化（复用 GC 的根扫描）
- 栈帧序列化
- Snapshot 格式定义与读写
- 基本的 `yield` 关键字（无 I/O、无并发）
- 跨架构恢复（x86 ↔ ARM，验证浮点一致性）

**交付物**：一个纯计算程序可以在 yield 后保存，在同架构或不同架构上 resume。

### Phase 3：I/O 系统（约 4–5 周）

**目标**：带 I/O 的程序可挂起和恢复。

- 内置 I/O 指令集（file_open, file_read, tcp_connect, http_get 等）
- I/O 句柄的生命周期管理
- 三类 I/O 的 snapshot 捕获逻辑
- 重连阶段实现（Replay / Seek / Cached）
- `yield resume { ... }` 语法支持

**交付物**：一个读文件、发 HTTP 请求的程序可以在 I/O 操作之间 yield，恢复后自动重连。

### Phase 4：结构化并发（约 3–4 周）

**目标**：多任务程序的挂起。

- `spawn` 关键字，任务树数据结构
- 父任务等待子任务完成
- 子任务完成 → 父任务 yield 的语义
- 并发 I/O（多个子任务各自做 I/O）

**交付物**：多个并发子任务各自抓取数据，父任务汇总后 yield。

### Phase 5：语言前端（约 4–6 周）

**目标**：可读可写的 Pausible 语法。

- 词法分析器 + 递归下降解析器
- 类型检查器（Hindley-Milner 或简化版）
- 源码 → 字节码编译器
- 友好的错误信息

**交付物**：开发者可以写 `.pau` 源文件，编译为字节码，在 VM 上运行。

### Phase 6：打磨与工具链（持续）

- Snapshot 的跨设备传输（文件 or 网络）
- 重连失败的高级处理策略（retry with backoff、降级、fallback）
- 调试器（可以附着到 VM、检查任务树）
- 代码版本管理与 snapshot 兼容性（v2）

---

## 四、潜在风险与缓解

| 风险 | 缓解 |
|---|---|
| I/O 重建不够透明，程序员难调试 | 重连阶段提供详细的 `ReconnectReport`，每个句柄有独立状态码和错误信息 |
| 浮点数跨架构非确定性 | 原则上 IEEE 754 是确定的；在 snapshot 中存原始字节，不依赖宿主架构的浮点语义 |
| 大 snapshot 序列化慢 | 增量 snapshot：只序列化自上次 snapshot 以来变化的对象；用写屏障追踪脏对象 |
| 子任务必须完成的限制太死 | Phase 4 先做这个安全版本，Phase 4.5 引入子任务协同 yield |