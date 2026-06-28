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
> **仅余 `TimerSleep` 为占位符（no-op），计划在 3.5 重连阶段补充。

`OpCode` 枚举新增以下 I/O 指令：


| 类别 | 指令 | VM 执行状态 |
|---|---|---|
| 文件 | `FileOpen(path, mode)`, `FileRead(handle)`, `FileWrite(handle)`, `FileSeek(handle, offset)`, `FileClose(handle)` | ✅ 真实 `std::fs` |
| 网络 | `TcpConnect(addr)`, `TcpRead(handle)`, `TcpWrite(handle)`, `TcpClose(handle)` | ✅ `std::net::TcpStream` |
| HTTP | `HttpGet(url)`, `HttpPost(url, body)` | ✅ `ureq` 客户端 |
| 标准流 | `StdinRead`, `StdoutWrite`, `StderrWrite` | ✅ 走 `IoHandle` 缓冲区 |
| 定时器 | `TimerSleep(ms)` | ⚠️ 占位符（见下方说明） |

**占位符详情：**

| 指令 | 当前行为 | 需在哪个阶段补全 |
|---|---|---|
| `TimerSleep` | 空操作，不 sleep | 3.5（重连阶段需要计时器支持） |

> **还未实现。** 当前 `Yield` 只设置 `running = false`，不触发句柄快照。此逻辑在 3.4 中实现。

### 3.3 三类 I/O 策略
 > **状态：已完成。**
>
| 类别 | 语义 | Snapshot 保存内容 | Resume 行为 |
|---|---|---|---|
| **Replayable** | 可重放 | 请求参数 + 上次响应 | 重新发起请求；若结果不同，触发 `DataDiverged` 事件 |
| **Seekable** | 可定位 | 路径 + 偏移量 | 重新打开 + seek；若文件不存在或变短，触发 `ResourceLost` |
| **Ephemeral** | 一次性 | 完整缓存的数据 | 直接使用 snapshot 中的缓存值 |

### 3.4 Snapshot 中的 I/O 句柄
 > **状态：已完成。**

- `IoHandleSnapshot` 结构：`id`, `kind`, `strategy`, `params` (重连参数), `cached` (Ephemeral 缓存), `position` (Seekable 偏移)
- `Snapshot::capture` 扩展：在标记根后遍历活跃 I/O 句柄，按策略序列化
- `SnapshotHeader` 新增 `io_handle_count` 字段
- 向后兼容：旧 snapshot（无 I/O 段）反序列化时 `io_handle_count=0` 应正常恢复
- **Yield 时自动触发 snapshot 捕获：** 修改 `OpCode::Yield` 分支，在设置 `running = false` 前调用 snapshot 逻辑

### 3.5 重连阶段
 > **状态：已完成。**
>
- `ReconnectReport` 结构：每个句柄一个 `ReconnectStatus`（`Ok` / `Degraded` / `Failed`），含错误信息和状态码
- `Snapshot::restore_into` 扩展：堆/帧/栈恢复后，进入重连阶段，逐句柄尝试重连
- `VM::resume` 行为：默认任何非 Optional 句柄重连失败则返回 `ResumeError::Reconnect`
- 为 `yield resume { ... }` 语法预留 `ReconnectPolicy`（Phase 5 编译器后可用）
- `OpCode::Yield` 后跟 `ResumeBlock`（`ok` / `partial` / `error` 三个分支地址），当前阶段可预留操作数位置，默认行为为"任一失败则终止"

### 3.6 测试与验证

> 以下测试依赖 3.4（Snapshot 中的 I/O 句柄）和 3.5（重连阶段），需待两者实现后方可运行。
> 3.1–3.3 已实现的独立功能测试已补充在本模块的单元测试中。

#### 3.6.1 Snapshot 与恢复测试

- [ ] **文件 yield 后自动重连读取**：在文件第 N 次 read 后 yield，snapshot 中保存路径+偏移量；resume 时 Seek 策略自动重开文件并 seek 到记录位置，验证后续 read 结果与不中断时一致。
- [ ] **HTTP GET 重放 — 响应一致路径**：对固定响应端点发 GET，yield 后 snapshot；resume 时 Replay 策略重新请求同一 URL，比对响应体一致，程序正常继续。
- [ ] **HTTP GET 重放 — 响应不一致路径**：对每次返回不同值的端点（或 mock）发 GET，yield 后 snapshot；resume 重放时响应不同，触发 `DataDiverged` 事件并反映在 `ReconnectReport` 中。
- [ ] **HTTP POST 重放**：对 POST 端点发送请求体，yield 后 snapshot；resume 时 Replay 策略重发相同请求体，验证正确记录 `last_request`/`last_response`。
- [ ] **Ephemeral 流缓存恢复**：向 stdin buffer 写入模拟数据，read 后 yield；resume 时 Cached 策略直接使用 snapshot 中的缓存值，不再读实际 stdin。
- [ ] **多类型句柄共存 snapshot**：同时持有 File（Seek）+ TcpStream（Replay）+ Stdin（Cached），yield 后 snapshot 正确捕获所有句柄；resume 后每种句柄按各自策略恢复，互不干扰。

#### 3.6.2 错误路径测试

- [ ] **文件不存在 → `ResourceLost`**：snapshot 中记录的 File 路径在 resume 前被删除，恢复时重开失败，`ReconnectReport` 报告该句柄 `Failed`。
- [ ] **文件变短 → `ResourceLost`**：snapshot 记录 offset=100，但 resume 时文件只有 50 字节；seek 到 offset 100 失败，报告 `Failed` 或 `Degraded`。
- [ ] **TCP 连接断开 → `ResourceLost`**：snapshot 中记录的 TCP 对端在 resume 前关闭，重连失败，报告 `Failed`。
- [ ] **HTTP 端点不可达 → 错误报告**：snapshot 中记录的 URL 在 resume 时不可达，重放失败，报告 `Failed`。

#### 3.6.3 兼容性测试

- [ ] **空 I/O 句柄 snapshot 向后兼容**：用 Phase 2（无 I/O 段）格式的 snapshot 文件在 Phase 3 VM 中恢复，`io_handle_count=0` 应正常恢复，不报错。
- [ ] **新 snapshot 格式可被旧 VM 识别**（前向兼容）：新格式 snapshot 文件包含 I/O 段，确保 header 中的 `io_handle_count` 字段在旧 reader 中不会导致反序列化失败（至少能识别为不兼容版本）。

#### 3.6.4 已有独立测试（3.1–3.3 阶段）

3.1–3.3 已完成功能的单元测试已直接写在 `src/vm.rs`、`src/io.rs` 和 `src/chunk.rs` 的 `#[cfg(test)]` 中，覆盖：
- 句柄注册/检索/修改/关闭（`handle_count_starts_at_zero`、`close_handle_removes_from_registry` 等 9 个测试）
- 文件读/写/seek 操作（`file_read_from_handle`、`file_write_then_read`、`file_seek_tracks_position`、`seek_strategy_read_tracks_position`，共 4 个）
- FileOpen/FileClose opcode 执行（`file_open_opcode_creates_file_handle`、`file_open_parses_read_mode_from_string`、`file_close_opcode_removes_handle_and_pushes_bool`，共 3 个）
- 标准流读/写（`stdout_write_accumulates`、`stderr_write_accumulates`、`stdin_read_from_buffer`，共 3 个）
- Cached 策略行为（`cached_strategy_file_read_uses_cache`）
- IoHandle::Clone 行为（`clone_file_sets_file_to_none`、`clone_tcp_sets_stream_to_none`、`clone_http_preserves_all_fields`，共 3 个）
- TCP 本地回显（`tcp_echo_roundtrip` — `TcpListener::bind` 失败时 panic，不静默跳过）
- TimerSleep 占位符行为（`timer_sleep_is_noop`）
- I/O OpCode 二进制序列化往返（`function_roundtrip_with_io_opcodes`，位于 `src/chunk.rs`）

#### 3.6.5 测试执行说明

- 所有 Phase 3 独立测试随 `cargo test` 一同运行，当前共 151 个单元测试 + 13 个集成测试。
- TCP 测试需要本地网络权限，`TcpListener::bind` 失败时 panic（不静默跳过）。
- HTTP 测试（3.6.1 中 GET/POST 重放测试）依赖 `ureq` 的网络能力，需在有外网访问的环境中运行。

**依赖顺序**：3.1 -> 3.2 -> 3.3 -> 3.4 -> 3.5 -> 3.6。
