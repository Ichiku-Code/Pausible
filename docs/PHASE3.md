## Phase 3 路线图：I/O 系统

**目标**：带 I/O 的程序可挂起和恢复。Snapshot 捕获所有活跃 I/O 句柄的状态，
Resume 时按类别重建连接（Replay / Seek / Cached），并提供显式的重连结果报告。

### 3.1 I/O 句柄类型与生命周期
> **状态：已完成。**

- `IoHandle` 枚举：`File`, `TcpStream`, `HttpConnection`, `Timer`, `Stdin`, `Stdout`, `Stderr`
- 每个句柄携带 `IoStrategy` 标注（由程序员或编译器指定）：`Replay`, `Seek`, `Cached`
- VM 内部句柄注册表：`HashMap<HandleId, IoHandle>`，管理创建/销毁/snapshot
- 句柄的创建与关闭是指令驱动的（不经过 FFI），VM 拦截每一次 I/O 操作

### 3.2 I/O 指令集
> **状态：已完成。** 15 条指令的 OpCode 变体、mnemonic、Display、二进制序列化、VM 执行分支均已完成。
> **待补：** 部分 VM 执行后端目前为占位符，需在后续阶段替换为真实实现。

`OpCode` 枚举新增以下 I/O 指令：

| 类别 | 指令 | VM 执行状态 |
|---|---|---|
| 文件 | `FileOpen(path, mode)`, `FileRead(handle)`, `FileWrite(handle)`, `FileSeek(handle, offset)`, `FileClose(handle)` | ✅ 真实 `std::fs` |
| 网络 | `TcpConnect(addr)`, `TcpRead(handle)`, `TcpWrite(handle)`, `TcpClose(handle)` | ⚠️ 占位符（见下方说明） |
| HTTP | `HttpGet(url)`, `HttpPost(url, body)` | ⚠️ 占位符（见下方说明） |
| 标准流 | `StdinRead`, `StdoutWrite`, `StderrWrite` | ✅ 走 `IoHandle` 缓冲区 |
| 定时器 | `TimerSleep(ms)` | ⚠️ 占位符（见下方说明） |

**占位符详情：**

| 指令 | 当前行为 | 需在哪个阶段补全 |
|---|---|---|
| `TcpConnect` | 创建空 `TcpStream` 句柄，不建立真实连接 | 3.3（策略实现时补充 `std::net::TcpStream` 连接逻辑） |
| `TcpRead` | 总是推 `Null` 到栈 | 同上 |
| `TcpWrite` | 弹出数据但不发送 | 同上 |
| `HttpGet` | 总是推 `Null` 到栈 | 3.3（策略实现时补充 HTTP 客户端逻辑） |
| `HttpPost` | 总是推 `Null` 到栈 | 同上 |
| `TimerSleep` | 空操作，不 sleep | 3.5（重连阶段需要计时器支持） |

原有的 `OpCode::Yield` 语义扩展：yield 时自动触发 I/O 句柄的 snapshot 捕获。
> **还未实现。** 当前 `Yield` 只设置 `running = false`，不触发句柄快照。此逻辑在 3.4 中实现。

### 3.3 三类 I/O 策略
> **状态：待实现。**
>
> 需要在本阶段完成的工作：
> - 实现 `IoStrategy::Replay` / `Seek` / `Cached` 的行为逻辑
> - 替换 TcpConnect/TcpRead/TcpWrite 的占位符为真实 `std::net::TcpStream` 实现
> - 替换 HttpGet/HttpPost 的占位符为真实 HTTP 客户端实现
> - 为每个 I/O 操作记录请求参数 + 上次响应（供 3.4 snapshot 使用）

| 类别 | 语义 | Snapshot 保存内容 | Resume 行为 |
|---|---|---|---|
| **Replayable** | 可重放 | 请求参数 + 上次响应 | 重新发起请求；若结果不同，触发 `DataDiverged` 事件 |
| **Seekable** | 可定位 | 路径 + 偏移量 | 重新打开 + seek；若文件不存在或变短，触发 `ResourceLost` |
| **Ephemeral** | 一次性 | 完整缓存的数据 | 直接使用 snapshot 中的缓存值 |

### 3.4 Snapshot 中的 I/O 句柄
> **状态：待实现。**

- `IoHandleSnapshot` 结构：`id`, `kind`, `strategy`, `params` (重连参数), `cached` (Ephemeral 缓存), `position` (Seekable 偏移)
- `Snapshot::capture` 扩展：在标记根后遍历活跃 I/O 句柄，按策略序列化
- `SnapshotHeader` 新增 `io_handle_count` 字段
- 向后兼容：旧 snapshot（无 I/O 段）反序列化时 `io_handle_count=0` 应正常恢复
- **Yield 时自动触发 snapshot 捕获：** 修改 `OpCode::Yield` 分支，在设置 `running = false` 前调用 snapshot 逻辑

### 3.5 重连阶段
> **状态：待实现。**
>
> 需要在本阶段完成的工作：
> - 替换 `TimerSleep` 的占位符为真实计时实现（重连需要等待超时）

- `ReconnectReport` 结构：每个句柄一个 `ReconnectStatus`（`Ok` / `Degraded` / `Failed`），含错误信息和状态码
- `Snapshot::restore_into` 扩展：堆/帧/栈恢复后，进入重连阶段，逐句柄尝试重连
- `VM::resume` 行为：默认任何非 Optional 句柄重连失败则返回 `ResumeError::Reconnect`
- 为 `yield resume { ... }` 语法预留 `ReconnectPolicy`（Phase 5 编译器后可用）
- `OpCode::Yield` 后跟 `ResumeBlock`（`ok` / `partial` / `error` 三个分支地址），当前阶段可预留操作数位置，默认行为为"任一失败则终止"

### 3.6 测试与验证

- [ ] 文件读写程序在 I/O 操作之间 yield，恢复后自动重连并继续
- [ ] HTTP GET 程序 yield 后恢复，重放请求并比对响应（一致 / 不一致两条路径）
- [ ] Ephemeral 流（stdin 模拟）yield 后恢复，使用缓存数据继续
- [ ] 多个不同类别 I/O 句柄共存，snapshot 正确捕获和恢复
- [ ] 文件不存在/变短时的 `ResourceLost` 错误路径
- [ ] 空 I/O 句柄的 VM snapshot 向后兼容（与 Phase 2 snapshot 互读）

**依赖顺序**：3.1 -> 3.2 -> 3.3 -> 3.4 -> 3.5 -> 3.6。
