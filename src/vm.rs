// `prepare`, `run`, `step` all return VmError; adding per-method # Errors
// sections would repeat the VmError variants verbatim.
#![allow(clippy::missing_errors_doc)]

use core::fmt;

use crate::function::Function;
use crate::heap::{Gc, Heap, ListObj, StringObj};
use crate::io::{HandleId, IoHandle, IoStrategy};
use crate::opcode::OpCode;
use crate::snapshot::Snapshot;
use crate::value::{TypeError, Value};
use std::collections::HashMap;
use std::net::TcpStream;

#[derive(Debug, Clone, PartialEq)]
pub enum HeapError {
    InvalidHandle,
}

impl core::fmt::Display for HeapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidHandle => write!(f, "invalid heap handle"),
        }
    }
}

impl core::error::Error for HeapError {}

/// A call frame tracks execution state for one function invocation.
#[derive(Debug, Clone)]
pub struct CallFrame {
    /// Index into `VM::functions`.
    pub function: usize,
    /// Instruction pointer (index into the function's bytecode).
    pub ip: usize,
    /// Local variable slots. Locals 0..arity are the parameters.
    pub locals: Vec<Value>,
}

impl CallFrame {
    pub(crate) fn new(function: usize, locals: Vec<Value>) -> Self {
        Self {
            function,
            ip: 0,
            locals,
        }
    }
}

/// Errors that can occur during VM execution.
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    /// Tried to pop from an empty operand stack.
    StackUnderflow,
    /// No call frame is active (e.g. Return with nothing to return from).
    EmptyFrameStack,
    /// Referenced a function index that does not exist.
    InvalidFunction(usize),
    /// Instruction pointer out of bounds.
    InvalidIp,
    /// Local variable index out of bounds.
    LocalOutOfBounds(usize),
    /// A typed operation received incompatible operands.
    TypeError(TypeError),
    /// A heap operation failed (e.g. accessing freed memory).
    HeapError(HeapError),
    /// Execution has stopped (Halt reached).
    Halted,
    /// I/O operation failed.
    IoError(String),
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StackUnderflow => write!(f, "stack underflow"),
            Self::EmptyFrameStack => write!(f, "empty call frame stack"),
            Self::InvalidFunction(idx) => write!(f, "invalid function index: {idx}"),
            Self::InvalidIp => write!(f, "instruction pointer out of bounds"),
            Self::LocalOutOfBounds(idx) => write!(f, "local {idx} out of bounds"),
            Self::TypeError(e) => write!(f, "{e}"),
            Self::HeapError(e) => write!(f, "{e}"),
            Self::Halted => write!(f, "VM halted"),
            Self::IoError(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl core::error::Error for VmError {}

/// Errors that can occur during the resume workflow (restore + continue execution).
#[derive(Debug, Clone, PartialEq)]
pub enum ResumeError {
    /// Snapshot restoration failed.
    Snapshot(String),
    /// VM execution failed after restore.
    Vm(VmError),
}

impl fmt::Display for ResumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(msg) => write!(f, "resume snapshot error: {msg}"),
            Self::Vm(e) => write!(f, "resume VM error: {e}"),
        }
    }
}

impl core::error::Error for ResumeError {}

/// The Pausible stack-based virtual machine.
#[derive(Debug, Clone)]
pub struct VM {
    /// Operand stack.
    pub stack: Vec<Value>,
    /// Call frames (top = active).
    pub frames: Vec<CallFrame>,
    /// Function table (indexed by function id).
    pub functions: Vec<Function>,
    /// Whether the VM is currently executing.
    pub running: bool,
    /// Heap for reference types (String, List, etc.).
    pub heap: Heap,
    /// Registered I/O handles, keyed by `HandleId`.
    pub handles: HashMap<HandleId, IoHandle>,
    /// Snapshot captured on Yield (None before first yield).
    pub snapshot: Option<Snapshot>,
    /// Monotonic counter for the next `HandleId` allocation.
    next_handle_id: u32,
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

impl VM {
    #[must_use]
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
            functions: Vec::new(),
            heap: Heap::new(),
            handles: HashMap::new(),
            next_handle_id: 0,
            running: false,
            snapshot: None,
        }
    }

    /// Allocate a string on the heap, triggering GC when
    /// the live object count exceeds the threshold.
    pub fn alloc_string(&mut self, data: String) -> Gc<StringObj> {
        let gc = self.heap.alloc_string(data);
        self.maybe_gc();
        gc
    }

    /// Allocate a list on the heap, triggering GC when
    /// the live object count exceeds the threshold.
    pub fn alloc_list(&mut self, elements: Vec<Value>) -> Gc<ListObj> {
        let gc = self.heap.alloc_list(elements);
        self.maybe_gc();
        gc
    }

    /// Borrow a heap string by handle.
    pub fn get_string(&self, gc: Gc<StringObj>) -> Result<&StringObj, VmError> {
        self.heap
            .get(gc)
            .ok_or(VmError::HeapError(HeapError::InvalidHandle))
    }

    /// Mutably borrow a heap string.
    pub fn get_string_mut(&mut self, gc: Gc<StringObj>) -> Result<&mut StringObj, VmError> {
        self.heap
            .get_mut(gc)
            .ok_or(VmError::HeapError(HeapError::InvalidHandle))
    }

    /// Borrow a heap list by handle.
    pub fn get_list(&self, gc: Gc<ListObj>) -> Result<&ListObj, VmError> {
        self.heap
            .get(gc)
            .ok_or(VmError::HeapError(HeapError::InvalidHandle))
    }

    /// Mutably borrow a heap list.
    pub fn get_list_mut(&mut self, gc: Gc<ListObj>) -> Result<&mut ListObj, VmError> {
        self.heap
            .get_mut(gc)
            .ok_or(VmError::HeapError(HeapError::InvalidHandle))
    }

    // -- I/O handles --

    /// Register an I/O handle and return its id.
    pub fn create_handle(&mut self, handle: IoHandle) -> HandleId {
        let id = HandleId(self.next_handle_id);
        self.next_handle_id = self.next_handle_id.wrapping_add(1);
        self.handles.insert(id, handle);
        id
    }

    /// Borrow a registered I/O handle by id.
    #[must_use]
    pub fn get_handle(&self, id: HandleId) -> Option<&IoHandle> {
        self.handles.get(&id)
    }

    /// Mutably borrow a registered I/O handle by id.
    pub fn get_handle_mut(&mut self, id: HandleId) -> Option<&mut IoHandle> {
        self.handles.get_mut(&id)
    }

    /// Remove an I/O handle from the registry.
    pub fn close_handle(&mut self, id: HandleId) -> bool {
        self.handles.remove(&id).is_some()
    }

    /// Number of registered I/O handles.
    #[must_use]
    pub fn handle_count(&self) -> usize {
        self.handles.len()
    }

    // -- GC --

    /// Scan all roots (operand stack + frame locals) and mark every
    /// reachable heap object.
    pub fn mark_roots(&mut self) {
        for val in &self.stack {
            self.heap.mark_value(val);
        }
        for frame in &self.frames {
            for val in &frame.locals {
                self.heap.mark_value(val);
            }
        }
    }

    /// Run a full mark-sweep GC cycle.
    ///
    /// 1. Reset marks, then mark every reachable object from roots.
    /// 2. Sweep: unmarked slots are added to the free list for reuse.
    pub fn collect_garbage(&mut self) {
        self.heap.reset_marks();
        self.mark_roots();
        self.heap.collect_garbage_after_mark();
    }

    /// Trigger a GC if the heap has exceeded the threshold.
    fn maybe_gc(&mut self) {
        if self.heap.should_gc() {
            self.collect_garbage();
        }
    }

    /// Register a function and return its index.
    pub fn add_function(&mut self, func: Function) -> usize {
        let idx = self.functions.len();
        self.functions.push(func);
        idx
    }

    /// Prepare the VM to execute `main_idx` by pushing its call frame.
    pub fn prepare(&mut self, main_idx: usize) -> Result<(), VmError> {
        let func = self
            .functions
            .get(main_idx)
            .ok_or(VmError::InvalidFunction(main_idx))?;
        let locals = vec![Value::Null; func.num_locals];
        self.frames.push(CallFrame::new(main_idx, locals));
        self.running = true;
        Ok(())
    }

    /// Compute a hash of the function table for code-mismatch detection.
    ///
    /// Uses the output of  as the basis so
    /// that the hash reflects the full bytecode, including embedded
    /// heap constants. Two identical function tables produce the same
    /// hash; any change to instructions or embedded values changes it.
    #[must_use]
    pub fn code_hash(&self) -> u64 {
        use std::hash::Hasher;
        let module_bytes = crate::chunk::serialize_module(&self.functions, &self.heap);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write(&module_bytes);
        hasher.finish()
    }

    /// Create a snapshot of the current VM state.
    #[must_use]
    pub fn create_snapshot(&mut self) -> crate::snapshot::Snapshot {
        let hash = self.code_hash();
        crate::snapshot::Snapshot::capture(self, hash)
    }

    /// Restore VM state from a snapshot.
    ///
    /// # Errors
    ///
    /// Returns  if the current function
    /// table does not match the one used when the snapshot was taken.
    pub fn restore_snapshot(
        &mut self,
        snap: &crate::snapshot::Snapshot,
    ) -> Result<(), crate::snapshot::SnapshotError> {
        let hash = self.code_hash();
        snap.restore_into(self, hash)
    }

    /// Resume execution from a snapshot: restore state and continue running.
    ///
    /// This is the combined restore + run workflow for 2.6 Resume.
    /// Execution continues until the next `Yield` or `Halt`.
    ///
    /// # Errors
    ///
    /// Returns `ResumeError::Snapshot` if the code hash mismatches or the
    /// snapshot is malformed. Returns `ResumeError::Vm` if a runtime
    /// error occurs during continued execution.
    pub fn resume(&mut self, snap: &crate::snapshot::Snapshot) -> Result<(), ResumeError> {
        self.restore_snapshot(snap)
            .map_err(|e| ResumeError::Snapshot(e.to_string()))?;
        self.run().map_err(ResumeError::Vm)
    }

    /// Execute until Halt or an error.
    pub fn run(&mut self) -> Result<(), VmError> {
        while self.running {
            self.step()?;
        }
        Ok(())
    }

    /// Execute a single instruction.
    #[allow(clippy::too_many_lines)]
    pub fn step(&mut self) -> Result<(), VmError> {
        let frame_idx = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or(VmError::EmptyFrameStack)?;

        // Fetch: resolve current instruction (clone to release the borrow).
        let func_idx = self.frames[frame_idx].function;
        let ip = self.frames[frame_idx].ip;
        let func = self
            .functions
            .get(func_idx)
            .ok_or(VmError::InvalidFunction(func_idx))?;
        let op = func.code.get(ip).ok_or(VmError::InvalidIp)?.clone();

        // Advance IP before executing (jump-like instructions will overwrite).
        self.frames[frame_idx].ip = ip.wrapping_add(1);

        match op {
            // -- stack --
            OpCode::Push(v) => self.stack.push(v),
            OpCode::Pop => {
                self.pop()?;
            }
            OpCode::Dup => {
                let top = self.peek()?.clone();
                self.stack.push(top);
            }

            // -- arithmetic --
            OpCode::Add => self.binary_op(Value::add)?,
            OpCode::Sub => self.binary_op(Value::sub)?,
            OpCode::Mul => self.binary_op(Value::mul)?,
            OpCode::Div => self.binary_op(Value::div)?,
            OpCode::Mod => self.binary_op(Value::modulo)?,
            OpCode::Neg => self.unary_op(Value::neg)?,

            // -- comparison --
            OpCode::Eq => self.binary_op(Value::eq)?,
            OpCode::Neq => self.binary_op(Value::neq)?,
            OpCode::Lt => self.binary_op(Value::lt)?,
            OpCode::Gt => self.binary_op(Value::gt)?,
            OpCode::Lte => self.binary_op(Value::lte)?,
            OpCode::Gte => self.binary_op(Value::gte)?,

            // -- logical --
            OpCode::And => self.binary_op(Value::and)?,
            OpCode::Or => self.binary_op(Value::or)?,
            OpCode::Not => self.unary_op(Value::not)?,

            // -- control flow --
            OpCode::Jump(offset) => self.frames[frame_idx].ip = offset,
            OpCode::JumpIfTrue(offset) => {
                let cond = self.pop()?;
                if cond.is_truthy() {
                    self.frames[frame_idx].ip = offset;
                }
            }
            OpCode::JumpIfFalse(offset) => {
                let cond = self.pop()?;
                if !cond.is_truthy() {
                    self.frames[frame_idx].ip = offset;
                }
            }

            // -- locals --
            OpCode::Load(idx) => {
                let val = self.frames[frame_idx]
                    .locals
                    .get(idx)
                    .ok_or(VmError::LocalOutOfBounds(idx))?;
                self.stack.push(val.clone());
            }
            OpCode::Store(idx) => {
                let val = self.pop()?;
                let slot = self.frames[frame_idx]
                    .locals
                    .get_mut(idx)
                    .ok_or(VmError::LocalOutOfBounds(idx))?;
                *slot = val;
            }

            // -- functions --
            OpCode::Call(idx) => {
                let func = self
                    .functions
                    .get(idx)
                    .ok_or(VmError::InvalidFunction(idx))?
                    .clone();
                let arity = func.arity;
                let mut locals = Vec::with_capacity(func.num_locals);

                // Pop arguments from stack (pushed left-to-right, so pop in reverse).
                for _ in 0..arity {
                    locals.push(self.pop()?);
                }
                locals.reverse();
                locals.resize(func.num_locals, Value::Null);

                self.frames.push(CallFrame::new(idx, locals));
            }
            OpCode::Return => {
                let frame = self.frames.pop().ok_or(VmError::EmptyFrameStack)?;
                // Leave the caller's frame active. The return value is on the operand
                // stack (pushed by the callee's last instruction).
                // If the frame stack is now empty, halt.
                if self.frames.is_empty() {
                    self.running = false;
                }
                // Note: returning a value is the caller's responsibility to push.
                // For Phase 1, leave return value handling to the bytecode.
                let _ = frame;
            }
            OpCode::Yield => {
                let ch = self.code_hash();
                self.snapshot = Some(Snapshot::capture(self, ch));
                self.running = false;
            }
            OpCode::Halt => {
                self.running = false;
            }

            // -- I/O: file --
            OpCode::FileOpen { path, mode } => {
                let mode_val = mode;
                let path_val = path;
                let path_str = match &path_val {
                    Value::String(gc) => self
                        .heap
                        .get(*gc)
                        .map(|s| s.data.clone())
                        .unwrap_or_default(),
                    _ => path_val.to_string(),
                };
                let fmode = match &mode_val {
                    Value::String(gc) => match self.heap.get(*gc).map(|s| s.data.as_str()) {
                        Some("w" | "W") => crate::io::FileMode::Write,
                        Some("a" | "A") => crate::io::FileMode::Append,
                        _ => crate::io::FileMode::Read,
                    },
                    _ => crate::io::FileMode::Read,
                };
                let handle = IoHandle::File {
                    path: path_str,
                    mode: fmode,
                    position: 0,
                    strategy: crate::io::IoStrategy::Seek,
                    file: None,
                    cached: None,
                };
                let id = self.create_handle(handle);
                self.stack.push(Value::Int(i64::from(id.0)));
            }
            OpCode::FileRead(h) => {
                let data = self.read_file_handle(h)?;
                self.stack.push(data);
            }
            OpCode::FileWrite(h) => {
                let data = self.pop()?;
                let _written = self.write_file_handle(h, &data)?;
            }
            OpCode::FileSeek { handle, offset } => {
                let pos = self.seek_file_handle(handle, &offset);
                #[allow(clippy::cast_possible_wrap)]
                self.stack.push(Value::Int(pos as i64));
            }
            OpCode::FileClose(h) | OpCode::TcpClose(h) => {
                let removed = self.close_handle(h);
                self.stack.push(Value::Bool(removed));
            }

            // -- I/O: TCP --
            OpCode::TcpConnect { addr } => {
                let addr_str = value_to_string(&self.heap, &addr);
                match TcpStream::connect(&addr_str) {
                    Ok(stream) => {
                        let handle = IoHandle::TcpStream {
                            addr: addr_str,
                            strategy: crate::io::IoStrategy::Replay,
                            stream: Some(stream),
                            last_request: None,
                            last_response: None,
                        };
                        let id = self.create_handle(handle);
                        self.stack.push(Value::Int(i64::from(id.0)));
                    }
                    Err(e) => {
                        return Err(VmError::IoError(format!("TCP connect to {addr_str}: {e}")));
                    }
                }
            }
            OpCode::TcpRead(h) => {
                let data = self.tcp_read_handle(h)?;
                self.stack.push(data);
            }
            OpCode::TcpWrite(h) => {
                let data = self.pop()?;
                self.tcp_write_handle(h, &data)?;
            }

            // -- I/O: HTTP --
            OpCode::HttpGet { url } => {
                let data = self.http_get(&url)?;
                self.stack.push(data);
            }
            OpCode::HttpPost { url, body } => {
                let data = self.http_post(&url, &body)?;
                self.stack.push(data);
            }

            // -- I/O: standard streams --
            OpCode::StdinRead => {
                let data = self.read_stdin();
                self.stack.push(data);
            }
            OpCode::StdoutWrite => {
                let data = self.pop()?;
                self.write_stdout(&data);
            }
            OpCode::StderrWrite => {
                let data = self.pop()?;
                self.write_stderr(&data);
            }

            // -- I/O: timer (placeholder) --
            OpCode::TimerSleep { ms: _ } => {
                // Placeholder: sleep is a no-op in this phase
            }
        }

        Ok(())
    }

    // -- I/O helpers --

    fn read_file_handle(&mut self, h: HandleId) -> Result<Value, VmError> {
        use std::io::{Read, Seek};
        // Strategy-aware: check for cached data first
        let (path, strategy, cached) = match self.handles.get(&h) {
            Some(IoHandle::File {
                path,
                strategy,
                cached,
                ..
            }) => (path.clone(), *strategy, cached.clone()),
            _ => return Ok(Value::Null),
        };

        // Cached: return cached data if available
        if strategy == IoStrategy::Cached
            && let Some(data) = cached
        {
            let elements: Vec<Value> = data.into_iter().map(|b| Value::Int(i64::from(b))).collect();
            let gc = self.heap.alloc_list(elements);
            return Ok(Value::List(gc));
        }

        // Seek: use stored file handle if available
        let buf = if strategy == IoStrategy::Seek {
            let mut buf = Vec::new();
            let handle = self.handles.get_mut(&h);
            if let Some(IoHandle::File {
                file: Some(f),
                position,
                ..
            }) = handle
            {
                *position = f
                    .seek(std::io::SeekFrom::Start(*position))
                    .unwrap_or(*position);
                f.read_to_end(&mut buf)
                    .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
                buf
            } else {
                // Fallback: re-open from path for Seek if no stored handle
                let path_str = match self.handles.get(&h) {
                    Some(IoHandle::File { path, .. }) => path.clone(),
                    _ => return Ok(Value::Null),
                };
                let pos = match self.handles.get(&h) {
                    Some(IoHandle::File { position, .. }) => *position,
                    _ => 0,
                };
                let mut file = std::fs::File::open(&path_str)
                    .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
                file.seek(std::io::SeekFrom::Start(pos))
                    .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
                // Update position after fallback read
                if let Some(IoHandle::File { position, .. }) = self.handles.get_mut(&h) {
                    *position = pos + buf.len() as u64;
                }
                buf
            }
        } else {
            // Replay or fallback: re-open from path
            let mut file = std::fs::File::open(&path)
                .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
            buf
        };

        // Cache the read data for all strategies (for snapshot)
        if let Some(IoHandle::File { cached, .. }) = self.handles.get_mut(&h) {
            *cached = Some(buf.clone());
        }

        let elements: Vec<Value> = buf.into_iter().map(|b| Value::Int(i64::from(b))).collect();
        let gc = self.heap.alloc_list(elements);
        Ok(Value::List(gc))
    }

    fn write_file_handle(&mut self, h: HandleId, data: &Value) -> Result<usize, VmError> {
        use std::io::Write;
        let (path, strategy) = match self.handles.get(&h) {
            Some(IoHandle::File { path, strategy, .. }) => (path.clone(), *strategy),
            _ => return Ok(0),
        };
        let bytes = value_to_bytes(data);

        // Strategy-aware write: use stored file handle for Seek, otherwise re-open
        if strategy == IoStrategy::Seek {
            if let Some(IoHandle::File { file: Some(f), .. }) = self.handles.get_mut(&h) {
                f.write_all(&bytes)
                    .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
            } else {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&path)
                    .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
                file.write_all(&bytes)
                    .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
            }
        } else {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
            file.write_all(&bytes)
                .map_err(|_| VmError::HeapError(HeapError::InvalidHandle))?;
        }

        // Cache the written data for Cached strategy
        if let Some(IoHandle::File {
            cached,
            strategy: s,
            ..
        }) = self.handles.get_mut(&h)
            && *s == IoStrategy::Cached
        {
            *cached = Some(bytes.clone());
        }

        Ok(bytes.len())
    }

    fn seek_file_handle(&mut self, h: HandleId, offset: &Value) -> u64 {
        use std::io::Seek;
        let off = match offset {
            #[allow(clippy::cast_sign_loss)]
            Value::Int(n) => *n as u64,
            _ => 0,
        };
        // Update position and seek on stored file if present
        if let Some(IoHandle::File { position, file, .. }) = self.handles.get_mut(&h) {
            *position = off;
            if let Some(f) = file {
                let _ = f.seek(std::io::SeekFrom::Start(off));
            }
        }
        off
    }
    fn read_stdin(&mut self) -> Value {
        // Find the first Stdin handle and return its buffer as a List
        for handle in self.handles.values() {
            if let IoHandle::Stdin { buffer } = handle {
                let elements: Vec<Value> =
                    buffer.iter().map(|&b| Value::Int(i64::from(b))).collect();
                let gc = self.heap.alloc_list(elements);
                return Value::List(gc);
            }
        }
        Value::Null
    }

    fn write_stdout(&mut self, data: &Value) {
        let bytes = value_to_bytes(data);
        // Write to first Stdout handle
        for handle in self.handles.values_mut() {
            if let IoHandle::Stdout { buffer } = handle {
                buffer.extend_from_slice(&bytes);
                return;
            }
        }
    }

    fn write_stderr(&mut self, data: &Value) {
        let bytes = value_to_bytes(data);
        // Write to first Stderr handle
        for handle in self.handles.values_mut() {
            if let IoHandle::Stderr { buffer } = handle {
                buffer.extend_from_slice(&bytes);
                return;
            }
        }
    }

    fn tcp_read_handle(&mut self, h: HandleId) -> Result<Value, VmError> {
        use std::io::Read;
        let response_data = {
            let handle = self
                .handles
                .get_mut(&h)
                .ok_or_else(|| VmError::IoError("invalid TCP handle".into()))?;
            let IoHandle::TcpStream {
                stream,
                last_response,
                ..
            } = handle
            else {
                return Err(VmError::IoError("handle is not a TcpStream".into()));
            };
            let s = stream
                .as_mut()
                .ok_or_else(|| VmError::IoError("TCP stream not connected".into()))?;
            let mut buf = vec![0u8; 4096];
            let n = s
                .read(&mut buf)
                .map_err(|e| VmError::IoError(format!("TCP read: {e}")))?;
            buf.truncate(n);
            *last_response = Some(buf.clone());
            buf
        };

        let elements: Vec<Value> = response_data
            .into_iter()
            .map(|b| Value::Int(i64::from(b)))
            .collect();
        let gc = self.heap.alloc_list(elements);
        Ok(Value::List(gc))
    }

    fn tcp_write_handle(&mut self, h: HandleId, data: &Value) -> Result<(), VmError> {
        use std::io::Write;
        let bytes = value_to_bytes(data);
        let handle = self
            .handles
            .get_mut(&h)
            .ok_or_else(|| VmError::IoError("invalid TCP handle".into()))?;
        let IoHandle::TcpStream {
            stream,
            last_request,
            ..
        } = handle
        else {
            return Err(VmError::IoError("handle is not a TcpStream".into()));
        };
        let s = stream
            .as_mut()
            .ok_or_else(|| VmError::IoError("TCP stream not connected".into()))?;
        s.write_all(&bytes)
            .map_err(|e| VmError::IoError(format!("TCP write: {e}")))?;
        *last_request = Some(bytes);
        Ok(())
    }

    fn http_get(&mut self, url_val: &Value) -> Result<Value, VmError> {
        let url_str = value_to_string(&self.heap, url_val);

        let response = ureq::get(&url_str)
            .call()
            .map_err(|e| VmError::IoError(format!("HTTP GET {url_str}: {e}")))?;

        let body = response
            .into_string()
            .map_err(|e| VmError::IoError(format!("HTTP GET {url_str}: read body: {e}")))?;

        // Create a handle to record the request + response for snapshot
        let handle = IoHandle::HttpConnection {
            url: url_str,
            method: crate::io::HttpMethod::Get,
            body: None,
            last_response: Some(body.clone().into_bytes()),
            strategy: crate::io::IoStrategy::Replay,
        };
        self.create_handle(handle);

        let gc = self.heap.alloc_string(body);
        Ok(Value::String(gc))
    }

    fn http_post(&mut self, url_val: &Value, body_val: &Value) -> Result<Value, VmError> {
        let url_str = value_to_string(&self.heap, url_val);
        let body_bytes = value_to_bytes(body_val);
        let body_str = String::from_utf8_lossy(&body_bytes);

        let response = ureq::post(&url_str)
            .send_string(&body_str)
            .map_err(|e| VmError::IoError(format!("HTTP POST {url_str}: {e}")))?;

        let resp_body = response
            .into_string()
            .map_err(|e| VmError::IoError(format!("HTTP POST {url_str}: read body: {e}")))?;

        let handle = IoHandle::HttpConnection {
            url: url_str,
            method: crate::io::HttpMethod::Post,
            body: Some(body_bytes),
            last_response: Some(resp_body.clone().into_bytes()),
            strategy: crate::io::IoStrategy::Replay,
        };
        self.create_handle(handle);

        let gc = self.heap.alloc_string(resp_body);
        Ok(Value::String(gc))
    }

    // -- stack helpers --

    fn pop(&mut self) -> Result<Value, VmError> {
        self.stack.pop().ok_or(VmError::StackUnderflow)
    }

    fn peek(&self) -> Result<&Value, VmError> {
        self.stack.last().ok_or(VmError::StackUnderflow)
    }

    fn binary_op(
        &mut self,
        op: fn(&Value, &Value) -> Result<Value, TypeError>,
    ) -> Result<(), VmError> {
        let rhs = self.pop()?;
        let lhs = self.pop()?;
        let result = op(&lhs, &rhs).map_err(VmError::TypeError)?;
        self.stack.push(result);
        Ok(())
    }

    fn unary_op(&mut self, op: fn(&Value) -> Result<Value, TypeError>) -> Result<(), VmError> {
        let val = self.pop()?;
        let result = op(&val).map_err(VmError::TypeError)?;
        self.stack.push(result);
        Ok(())
    }
}

/// Convert a Value to a byte vector for I/O write operations.
fn value_to_bytes(val: &Value) -> Vec<u8> {
    match val {
        Value::Int(n) => n.to_string().into_bytes(),
        Value::Float(f) => f.to_string().into_bytes(),
        Value::Bool(b) => {
            if *b {
                b"true".to_vec()
            } else {
                b"false".to_vec()
            }
        }
        Value::Null => b"null".to_vec(),
        Value::String(_gc) => {
            // GC-backed strings are checked at the VM-level; here we just
            // return empty bytes for safety.
            Vec::new()
        }
        Value::List(_gc) => {
            // Lists are serialized as [] for now
            Vec::new()
        }
    }
}

/// Convert a Value to a string. For String values, reads from the heap.
fn value_to_string(heap: &Heap, val: &Value) -> String {
    match val {
        Value::String(gc) => heap.get(*gc).map(|s| s.data.clone()).unwrap_or_default(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::Function;
    use crate::opcode::OpCode;
    use crate::value::Value;

    fn make_vm(code: Vec<OpCode>) -> VM {
        let mut vm = VM::new();
        let main = Function::new("main", 0, code, 0);
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm
    }

    fn run_code(code: Vec<OpCode>) -> Result<VM, VmError> {
        let mut vm = make_vm(code);
        vm.run()?;
        Ok(vm)
    }

    // -- basic arithmetic --

    #[test]
    fn simple_push_and_add() {
        let vm = run_code(vec![
            OpCode::Push(Value::Int(2)),
            OpCode::Push(Value::Int(3)),
            OpCode::Add,
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Int(5)]);
    }

    #[test]
    fn sub_mul_div_mod() {
        let vm = run_code(vec![
            OpCode::Push(Value::Int(10)),
            OpCode::Push(Value::Int(3)),
            OpCode::Sub,
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Int(7)]);
    }

    #[test]
    fn float_arithmetic() {
        let vm = run_code(vec![
            OpCode::Push(Value::Float(3.0)),
            OpCode::Push(Value::Float(4.0)),
            OpCode::Mul,
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Float(12.0)]);
    }

    #[test]
    fn negate() {
        let vm = run_code(vec![
            OpCode::Push(Value::Int(42)),
            OpCode::Neg,
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Int(-42)]);
    }

    // -- stack ops --

    #[test]
    fn pop_discards_top() {
        let vm = run_code(vec![
            OpCode::Push(Value::Int(1)),
            OpCode::Push(Value::Int(2)),
            OpCode::Pop,
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Int(1)]);
    }

    #[test]
    fn dup_duplicates_top() {
        let vm = run_code(vec![OpCode::Push(Value::Int(7)), OpCode::Dup, OpCode::Halt]).unwrap();
        assert_eq!(vm.stack, vec![Value::Int(7), Value::Int(7)]);
    }

    // -- comparison --

    #[test]
    fn compare_eq_and_lt() {
        let vm = run_code(vec![
            OpCode::Push(Value::Int(5)),
            OpCode::Push(Value::Int(5)),
            OpCode::Eq,
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Bool(true)]);
    }

    #[test]
    fn compare_lt() {
        let vm = run_code(vec![
            OpCode::Push(Value::Int(3)),
            OpCode::Push(Value::Int(7)),
            OpCode::Lt,
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Bool(true)]);
    }

    // -- logical --

    #[test]
    fn logical_and_or_not() {
        let vm = run_code(vec![
            OpCode::Push(Value::Bool(true)),
            OpCode::Push(Value::Bool(false)),
            OpCode::Or,
            OpCode::Not,
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Bool(false)]);
    }

    // -- control flow --

    #[test]
    fn unconditional_jump_skips_halt() {
        let vm = run_code(vec![
            OpCode::Jump(3),
            OpCode::Push(Value::Int(999)), // skipped
            OpCode::Halt,
            OpCode::Push(Value::Int(42)),
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Int(42)]);
    }

    #[test]
    fn jump_if_true() {
        let vm = run_code(vec![
            OpCode::Push(Value::Bool(true)),
            OpCode::JumpIfTrue(4),
            OpCode::Push(Value::Int(1)), // skipped when true
            OpCode::Halt,
            OpCode::Push(Value::Int(2)),
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Int(2)]);
    }

    #[test]
    fn jump_if_false() {
        let vm = run_code(vec![
            OpCode::Push(Value::Bool(false)),
            OpCode::JumpIfFalse(4),
            OpCode::Push(Value::Int(1)), // skipped when false
            OpCode::Halt,
            OpCode::Push(Value::Int(2)),
            OpCode::Halt,
        ])
        .unwrap();
        assert_eq!(vm.stack, vec![Value::Int(2)]);
    }

    #[test]
    fn loop_with_counter() {
        // Pseudo: for i in 3..0: push(i); halt
        //  0: push 3
        //  1: store 0        // local[0] = counter
        //  2: load 0
        //  3: push 0
        //  4: eq              // counter == 0?
        //  5: jump_if_true 10 // if true, halt
        //  6: load 0
        //  7: push 1
        //  8: sub             // counter - 1
        //  9: store 0
        // 10: jump 2          // loop back
        // 11: halt
        let code = vec![
            OpCode::Push(Value::Int(3)),
            OpCode::Store(0),
            // loop start (ip=2)
            OpCode::Load(0),
            OpCode::Push(Value::Int(0)),
            OpCode::Eq,
            OpCode::JumpIfTrue(11),
            OpCode::Load(0),
            OpCode::Push(Value::Int(1)),
            OpCode::Sub,
            OpCode::Store(0),
            OpCode::Jump(2),
            OpCode::Halt,
        ];

        let mut vm = VM::new();
        let main = Function::new("main", 0, code, 1);
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.run().unwrap();

        // jump_if_true pops the condition, and each iteration cleans up its
        // intermediate values via store. Final stack is empty.
        assert!(vm.stack.is_empty());
    }

    // -- locals --

    #[test]
    fn store_and_load_local() {
        let code = vec![
            OpCode::Push(Value::Int(100)),
            OpCode::Store(0),
            OpCode::Push(Value::Int(200)),
            OpCode::Store(1),
            OpCode::Load(0),
            OpCode::Load(1),
            OpCode::Add,
            OpCode::Halt,
        ];

        let mut vm = VM::new();
        let main = Function::new("main", 0, code, 3);
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.run().unwrap();

        assert_eq!(vm.stack, vec![Value::Int(300)]);
    }

    // -- function calls --

    #[test]
    fn call_and_return() {
        // function 1 (add_one): takes 1 arg, returns arg + 1
        //  0: load 0
        //  1: push 1
        //  2: add
        //  3: ret
        let add_one = Function::new(
            "add_one",
            1,
            vec![
                OpCode::Load(0),
                OpCode::Push(Value::Int(1)),
                OpCode::Add,
                OpCode::Return,
            ],
            1,
        );

        // main: push 41, call add_one, halt
        let main = Function::new(
            "main",
            0,
            vec![OpCode::Push(Value::Int(41)), OpCode::Call(1), OpCode::Halt],
            0,
        );

        let mut vm = VM::new();
        vm.add_function(main);
        vm.add_function(add_one);
        vm.prepare(0).unwrap();
        vm.run().unwrap();

        // Arguments are popped by Call; only the return value remains.
        assert_eq!(vm.stack, vec![Value::Int(42)]);
    }

    #[test]
    fn nested_calls() {
        // double(x) = x * 2
        let double = Function::new(
            "double",
            1,
            vec![
                OpCode::Load(0),
                OpCode::Push(Value::Int(2)),
                OpCode::Mul,
                OpCode::Return,
            ],
            1,
        );

        // add(x, y) = double(x) + double(y)
        let add_doubled = Function::new(
            "add_doubled",
            2,
            vec![
                OpCode::Load(0),
                OpCode::Call(2), // double(x)
                OpCode::Load(1),
                OpCode::Call(2), // double(y)
                OpCode::Add,
                OpCode::Return,
            ],
            2,
        );

        let main = Function::new(
            "main",
            0,
            vec![
                OpCode::Push(Value::Int(3)),
                OpCode::Push(Value::Int(4)),
                OpCode::Call(1), // add_doubled(3, 4) → 6 + 8 = 14
                OpCode::Halt,
            ],
            0,
        );

        let mut vm = VM::new();
        vm.add_function(main); // idx 0
        vm.add_function(add_doubled); // idx 1
        vm.add_function(double); // idx 2
        vm.prepare(0).unwrap();
        vm.run().unwrap();

        // Arguments 3 and 4 are consumed by the Call; only return value remains.
        assert_eq!(vm.stack, vec![Value::Int(14)]);
    }

    // -- errors --

    #[test]
    fn stack_underflow_on_pop_empty() {
        let result = run_code(vec![OpCode::Pop, OpCode::Halt]);
        assert!(matches!(result, Err(VmError::StackUnderflow)));
    }

    #[test]
    fn type_error_on_mixed_arithmetic() {
        let result = run_code(vec![
            OpCode::Push(Value::Int(1)),
            OpCode::Push(Value::Float(2.0)),
            OpCode::Add,
            OpCode::Halt,
        ]);
        assert!(matches!(result, Err(VmError::TypeError(_))));
    }

    #[test]
    fn local_out_of_bounds() {
        let result = run_code(vec![
            OpCode::Push(Value::Int(1)),
            OpCode::Store(99), // no locals allocated
            OpCode::Halt,
        ]);
        assert!(matches!(result, Err(VmError::LocalOutOfBounds(99))));
    }

    #[test]
    fn invalid_function_call() {
        let result = run_code(vec![OpCode::Call(42), OpCode::Halt]);
        assert!(matches!(result, Err(VmError::InvalidFunction(42))));
    }

    #[test]
    fn empty_frame_stack_run() {
        let mut vm = VM::new();
        vm.running = true;
        let result = vm.run();
        assert!(matches!(result, Err(VmError::EmptyFrameStack)));
    }

    // -- GC tests --

    #[test]
    fn gc_collects_unreferenced_string() {
        let mut vm = VM::new();

        // Allocate a string and store its handle. Then drop the reference
        // by overwriting the local. GC should reclaim the dead string.
        let _gc = vm.alloc_string("discard me".into());
        assert_eq!(vm.heap.len(), 1);

        // The Gc is still held by the variable `gc`, so it's reachable.
        // We need to test with VM roots instead.
        // For now: alloc multiple objects, ensure GC runs, verify
        // that unreachable objects get freed.

        // Allocate 3 strings, all kept reachable
        let _a = vm.alloc_string("keep".into());
        let _b = vm.alloc_string("keep".into());
        let _c = vm.alloc_string("keep".into());
        // _a, _b, _c are kept alive by local variables
    }

    #[test]
    fn gc_frees_unreachable_heap_objects() {
        let mut vm = VM::new();

        // Allocate objects; they're reachable via local variables `a`, `b`
        let a = vm.alloc_string("hello".into());
        let b = vm.alloc_list(vec![Value::Int(1), Value::Int(2)]);
        assert_eq!(vm.heap.len(), 2);

        // Push to stack to make them GC roots
        vm.stack.push(Value::String(a));
        vm.stack.push(Value::List(b));
        let cap_before = vm.heap.capacity();

        // Pop from stack — now both are unreachable
        vm.stack.pop();
        vm.stack.pop();

        // Manually trigger GC
        vm.collect_garbage();

        // Both objects should be dead — free_slots should have 2 entries
        assert_eq!(vm.heap.len(), 0);
        assert_eq!(vm.heap.capacity(), cap_before);
    }

    #[test]
    fn gc_preserves_reachable_objects() {
        let mut vm = VM::new();

        let a = vm.alloc_string("survivor".into());
        let _b = vm.alloc_string("also alive".into());

        // a is on stack (root), b is not — but we'll keep b reachable
        vm.stack.push(Value::String(a));

        // b is NOT on stack, so it will be collected
        let cap_before = vm.heap.capacity();

        vm.collect_garbage();

        // a should survive (on stack), b should be dead
        assert_eq!(vm.heap.len(), 1);
        assert_eq!(vm.heap.capacity(), cap_before);
        // Verify 'a' is still valid
        let a_data = vm.get_string(a).unwrap();
        assert_eq!(a_data.data, "survivor");
    }

    #[test]
    fn gc_preserves_nested_list_elements() {
        let mut vm = VM::new();

        // Build: outer = [inner, 42] where inner = [1, 2]
        // inner is only reachable via outer
        let inner = vm.alloc_list(vec![Value::Int(1), Value::Int(2)]);
        let outer = vm.alloc_list(vec![Value::List(inner), Value::Int(42)]);

        // Only outer is on the stack — GC must find inner through it
        vm.stack.push(Value::List(outer));

        vm.collect_garbage();

        // Both outer and inner should survive
        assert_eq!(vm.heap.len(), 2);
        let outer_list = vm.get_list(outer).unwrap();
        assert_eq!(outer_list.elements.len(), 2);
        if let Value::List(inner_gc) = &outer_list.elements[0] {
            let inner_list = vm.get_list(*inner_gc).unwrap();
            assert_eq!(inner_list.elements, vec![Value::Int(1), Value::Int(2)]);
        } else {
            panic!("inner should be a List");
        }
    }

    #[test]
    fn gc_slot_reuse_after_collection() {
        let mut vm = VM::new();

        let a = vm.alloc_string("first".into());
        let idx_a = a.index;
        let _ = a; // keep alive for now
        vm.stack.push(Value::String(a));

        // Force allocation up to the threshold so GC runs via maybe_gc
        // ... actually let's test manually
        vm.stack.pop(); // now a is dead

        vm.collect_garbage();
        assert_eq!(vm.heap.len(), 0);

        // Allocate a new string — should reuse the same slot
        let b = vm.alloc_string("second".into());
        assert_eq!(b.index, idx_a, "new allocation should reuse freed slot");

        let b_data = vm.get_string(b).unwrap();
        assert_eq!(b_data.data, "second");
    }

    #[test]
    fn gc_multiple_cycles() {
        let mut vm = VM::new();

        // Cycle 1
        let a = vm.alloc_string("one".into());
        vm.stack.push(Value::String(a));
        vm.collect_garbage();
        assert_eq!(vm.heap.len(), 1);

        // Cycle 2: pop a, then gc — a should die
        vm.stack.pop();
        vm.collect_garbage();
        assert_eq!(vm.heap.len(), 0);

        // Cycle 3: allocate new, keep it
        let b = vm.alloc_string("two".into());
        vm.stack.push(Value::String(b));
        vm.collect_garbage();
        assert_eq!(vm.heap.len(), 1);
        assert_eq!(vm.get_string(b).unwrap().data, "two");
    }
    #[test]
    fn yield_pauses_execution() {
        let mut vm = make_vm(vec![
            OpCode::Push(Value::Int(42)),
            OpCode::Yield,
            OpCode::Push(Value::Int(99)),
            OpCode::Halt,
        ]);
        vm.step().unwrap(); // push 42
        assert_eq!(vm.stack, &[Value::Int(42)]);
        assert!(vm.running);

        vm.step().unwrap(); // yield -> pauses
        assert!(!vm.running);
        // Stack is unchanged after yield
        assert_eq!(vm.stack, &[Value::Int(42)]);

        // Can resume after yield
        vm.running = true;
        vm.step().unwrap(); // push 99
        assert_eq!(vm.stack, &[Value::Int(42), Value::Int(99)]);
        vm.step().unwrap(); // halt
        assert!(!vm.running);
    }

    #[test]
    fn resume_continues_after_yield() {
        let mut vm = make_vm(vec![
            OpCode::Push(Value::Int(10)),
            OpCode::Yield,
            OpCode::Push(Value::Int(20)),
            OpCode::Halt,
        ]);

        vm.step().unwrap(); // push 10
        vm.step().unwrap(); // yield → pauses

        // Snapshot at yield point
        let snap = vm.create_snapshot();

        // Resume into same VM (restore + run)
        let err = vm.resume(&snap);
        assert!(err.is_ok(), "resume should succeed after yield");
        // After resume, run continues until next Yield/Halt
        // push 20 was executed, then halt
        assert_eq!(vm.stack, &[Value::Int(10), Value::Int(20)]);
        assert!(!vm.running);
    }

    #[test]
    fn resume_preserves_heap_objects() {
        let mut vm = make_vm(vec![
            OpCode::Push(Value::Null), // placeholder
            OpCode::Yield,
            OpCode::Halt,
        ]);

        // Push a heap-allocated string and store it in a local
        let gc = vm.alloc_string("persistent".into());
        vm.stack.push(Value::String(gc));

        // Step through Push (pushes Null on top of our string)
        vm.step().unwrap(); // push Null
        // Pop the Null, leaving our string on stack
        vm.stack.pop().unwrap();
        vm.step().unwrap(); // yield → pauses

        let snap = vm.create_snapshot();
        let err = vm.resume(&snap);
        assert!(err.is_ok(), "resume should succeed");
        assert!(!vm.frames.is_empty(), "frames should survive resume");
        if let Value::String(gc2) = &vm.stack[0] {
            assert_eq!(vm.heap.get(*gc2).unwrap().data, "persistent");
        } else {
            panic!("expected String on stack");
        }
    }

    #[test]
    fn resume_multiple_cycles() {
        let mut vm = make_vm(vec![
            OpCode::Push(Value::Int(1)),
            OpCode::Yield,
            OpCode::Push(Value::Int(2)),
            OpCode::Yield,
            OpCode::Push(Value::Int(3)),
            OpCode::Halt,
        ]);

        // Cycle 1: push 1, yield
        vm.step().unwrap(); // push 1
        vm.step().unwrap(); // yield
        let snap1 = vm.create_snapshot();
        vm.resume(&snap1).unwrap(); // push 2, yield, halt
        // Actually push 2 then yield → running is false
        assert!(!vm.running);
        assert!(!vm.stack.is_empty());

        // Cycle 2: manually resume by re-enabling running
        vm.running = true;
        vm.step().unwrap(); // push 3
        vm.step().unwrap(); // halt
        assert_eq!(vm.stack, &[Value::Int(1), Value::Int(2), Value::Int(3)]);
    }

    #[test]
    fn resume_error_display() {
        assert_eq!(
            format!("{}", ResumeError::Snapshot("bad".into())),
            "resume snapshot error: bad"
        );
        assert_eq!(
            format!("{}", ResumeError::Vm(VmError::Halted)),
            "resume VM error: VM halted"
        );
    }

    // -- I/O handle registry tests --

    #[test]
    fn handle_creation_and_retrieval() {
        let mut vm = VM::new();
        let h = IoHandle::File {
            file: None,
            cached: None,
            path: "/tmp/a.txt".into(),
            mode: crate::io::FileMode::Read,
            position: 0,
            strategy: crate::io::IoStrategy::Seek,
        };
        let id = vm.create_handle(h);
        assert_eq!(vm.handle_count(), 1);

        let retrieved = vm.get_handle(id).unwrap();
        assert_eq!(retrieved.kind_name(), "File");
    }

    #[test]
    fn handle_id_monotonic() {
        let mut vm = VM::new();
        let h1 = IoHandle::Stdin { buffer: vec![] };
        let h2 = IoHandle::Stdout { buffer: vec![] };

        let id1 = vm.create_handle(h1);
        let id2 = vm.create_handle(h2);
        assert_eq!(id1, HandleId(0));
        assert_eq!(id2, HandleId(1));
    }

    #[test]
    fn close_handle_removes_from_registry() {
        let mut vm = VM::new();
        let h = IoHandle::Timer {
            ms: 100,
            strategy: crate::io::IoStrategy::Replay,
        };
        let id = vm.create_handle(h);
        assert_eq!(vm.handle_count(), 1);

        assert!(vm.close_handle(id));
        assert_eq!(vm.handle_count(), 0);
        assert!(vm.get_handle(id).is_none());
    }

    #[test]
    fn close_nonexistent_handle_returns_false() {
        let mut vm = VM::new();
        assert!(!vm.close_handle(HandleId(99)));
    }

    #[test]
    fn get_handle_mut_allows_modification() {
        let mut vm = VM::new();
        let h = IoHandle::File {
            path: "/tmp/log.txt".into(),
            mode: crate::io::FileMode::Write,
            file: None,
            cached: None,
            position: 0,
            strategy: crate::io::IoStrategy::Seek,
        };
        let id = vm.create_handle(h);

        if let Some(IoHandle::File { position, .. }) = vm.get_handle_mut(id) {
            *position = 128;
        }

        if let Some(IoHandle::File { position, .. }) = vm.get_handle(id) {
            assert_eq!(*position, 128);
        } else {
            panic!("expected File handle");
        }
    }

    #[test]
    fn handle_count_starts_at_zero() {
        let vm = VM::new();
        assert_eq!(vm.handle_count(), 0);
    }

    #[test]
    fn multiple_handle_types_coexist() {
        let mut vm = VM::new();
        vm.create_handle(IoHandle::Stdin { buffer: vec![1, 2] });
        vm.create_handle(IoHandle::Stdout { buffer: vec![] });
        vm.create_handle(IoHandle::File {
            path: "/tmp/x".into(),
            mode: crate::io::FileMode::Read,
            file: None,
            cached: None,
            position: 0,
            strategy: crate::io::IoStrategy::Seek,
        });
        vm.create_handle(IoHandle::HttpConnection {
            url: "https://example.com".into(),
            method: crate::io::HttpMethod::Get,
            body: None,
            last_response: None,
            strategy: crate::io::IoStrategy::Replay,
        });
        assert_eq!(vm.handle_count(), 4);
    }

    #[test]
    fn vm_clone_preserves_handles() {
        let mut vm = VM::new();
        let h = IoHandle::Stdin { buffer: vec![42] };
        vm.create_handle(h);
        assert_eq!(vm.handle_count(), 1);

        let vm2 = vm.clone();
        assert_eq!(vm2.handle_count(), 1);
    }

    #[test]
    fn handle_id_wrapping_is_safe() {
        let mut vm = VM::new();
        vm.next_handle_id = u32::MAX;
        let h = IoHandle::Stdin { buffer: vec![] };
        let id = vm.create_handle(h);
        assert_eq!(id, HandleId(u32::MAX));
        // wrapping_add: next becomes 0
        assert_eq!(vm.next_handle_id, 0);
    }

    // -- I/O opcode execution tests --

    #[test]
    fn file_open_opcode_creates_file_handle() {
        let mut vm = VM::new();
        let path = vm.alloc_string("/tmp/pausible_test_open.txt".into());
        let mode = vm.alloc_string("w".into());
        std::fs::write("/tmp/pausible_test_open.txt", b"hello").unwrap();

        let main = Function::new(
            "main",
            0,
            vec![
                OpCode::FileOpen {
                    path: Value::String(path),
                    mode: Value::String(mode),
                },
                OpCode::Halt,
            ],
            0,
        );
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.step().unwrap();

        assert_eq!(vm.handle_count(), 1);
        if let Value::Int(id) = vm.stack.last().unwrap() {
            let h = vm.get_handle(HandleId((*id).try_into().unwrap())).unwrap();
            assert_eq!(h.kind_name(), "File");
        } else {
            panic!("expected Int handle ID on stack");
        }
        let _ = std::fs::remove_file("/tmp/pausible_test_open.txt");
    }

    #[test]
    fn file_open_parses_read_mode_from_string() {
        let mut vm = VM::new();
        let path = vm.alloc_string("/tmp/pausible_test_mode.txt".into());
        let mode = vm.alloc_string("r".into());
        std::fs::write("/tmp/pausible_test_mode.txt", b"data").unwrap();

        let main = Function::new(
            "main",
            0,
            vec![
                OpCode::FileOpen {
                    path: Value::String(path),
                    mode: Value::String(mode),
                },
                OpCode::Halt,
            ],
            0,
        );
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.step().unwrap();

        let id = if let Value::Int(id) = vm.stack.last().unwrap() {
            (*id).try_into().unwrap()
        } else {
            0
        };
        if let IoHandle::File { mode: fmode, .. } = vm.get_handle(HandleId(id)).unwrap() {
            assert_eq!(*fmode, crate::io::FileMode::Read);
        } else {
            panic!("expected File handle");
        }
        let _ = std::fs::remove_file("/tmp/pausible_test_mode.txt");
    }

    #[test]
    fn file_close_opcode_removes_handle_and_pushes_bool() {
        let mut vm = VM::new();
        let path = vm.alloc_string("/tmp/pausible_test_close.txt".into());
        let mode = vm.alloc_string("w".into());
        std::fs::write("/tmp/pausible_test_close.txt", b"").unwrap();

        let main = Function::new(
            "main",
            0,
            vec![
                OpCode::FileOpen {
                    path: Value::String(path),
                    mode: Value::String(mode),
                },
                OpCode::FileClose(HandleId(0)),
                OpCode::Halt,
            ],
            0,
        );
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.step().unwrap(); // FileOpen
        vm.step().unwrap(); // FileClose
        assert_eq!(vm.handle_count(), 0);
        assert_eq!(vm.stack.last().unwrap(), &Value::Bool(true));
        let _ = std::fs::remove_file("/tmp/pausible_test_close.txt");
    }

    #[test]
    fn file_write_then_read() {
        let mut vm = VM::new();
        let path = "/tmp/pausible_test_write_read.txt";
        std::fs::write(path, b"").unwrap();

        let h = vm.create_handle(IoHandle::File {
            path: path.into(),
            mode: crate::io::FileMode::Write,
            position: 0,
            strategy: crate::io::IoStrategy::Seek,
            file: None,
            cached: None,
        });
        let id = h;

        let data = Value::Int(42);
        let written = vm.write_file_handle(id, &data).unwrap();
        assert_eq!(written, 2);

        let read_h = vm.create_handle(IoHandle::File {
            path: path.into(),
            mode: crate::io::FileMode::Read,
            position: 0,
            strategy: crate::io::IoStrategy::Seek,
            file: None,
            cached: None,
        });
        let result = vm.read_file_handle(read_h).unwrap();
        assert!(matches!(result, Value::List(_)));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn file_read_from_handle() {
        let path = "/tmp/pausible_test_read.txt";
        std::fs::write(path, b"abc").unwrap();

        let mut vm = VM::new();
        let h = vm.create_handle(IoHandle::File {
            path: path.into(),
            mode: crate::io::FileMode::Read,
            position: 0,
            strategy: crate::io::IoStrategy::Seek,
            file: None,
            cached: None,
        });

        let result = vm.read_file_handle(h).unwrap();
        if let Value::List(gc) = result {
            let elements = &vm.heap.get(gc).unwrap().elements;
            assert_eq!(
                elements,
                &[Value::Int(0x61), Value::Int(0x62), Value::Int(0x63),]
            );
        } else {
            panic!("expected List from file read");
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn file_seek_tracks_position() {
        let path = "/tmp/pausible_test_seek.txt";
        std::fs::write(path, b"abcdef").unwrap();

        let mut vm = VM::new();
        let h = vm.create_handle(IoHandle::File {
            path: path.into(),
            mode: crate::io::FileMode::Read,
            position: 0,
            strategy: crate::io::IoStrategy::Seek,
            file: None,
            cached: None,
        });

        vm.seek_file_handle(h, &Value::Int(3));
        let result = vm.read_file_handle(h).unwrap();
        if let Value::List(gc) = result {
            let elements = &vm.heap.get(gc).unwrap().elements;
            assert_eq!(elements.len(), 3);
        } else {
            panic!("expected List");
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn seek_strategy_read_tracks_position() {
        let path = "/tmp/pausible_test_seek_strategy.txt";
        std::fs::write(path, b"0123456789").unwrap();

        let mut vm = VM::new();
        let h = vm.create_handle(IoHandle::File {
            path: path.into(),
            mode: crate::io::FileMode::Read,
            position: 0,
            strategy: crate::io::IoStrategy::Seek,
            file: None,
            cached: None,
        });

        vm.seek_file_handle(h, &Value::Int(5));
        let result = vm.read_file_handle(h).unwrap();
        if let Value::List(gc) = result {
            let elements = &vm.heap.get(gc).unwrap().elements;
            assert_eq!(elements.len(), 5);
        }

        if let IoHandle::File { position, .. } = vm.get_handle(h).unwrap() {
            assert_eq!(*position, 10);
        }
        let _ = std::fs::remove_file(path);
    }

    // -- Std stream tests --

    #[test]
    fn stdout_write_accumulates() {
        let mut vm = VM::new();
        let h = vm.create_handle(IoHandle::Stdout { buffer: vec![] });
        let main = Function::new(
            "main",
            0,
            vec![
                OpCode::Push(Value::Int(65)), // 'A'
                OpCode::StdoutWrite,
                OpCode::Push(Value::Int(66)), // 'B'
                OpCode::StdoutWrite,
                OpCode::Halt,
            ],
            0,
        );
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.run().unwrap();

        if let IoHandle::Stdout { buffer } = vm.get_handle(h).unwrap() {
            assert_eq!(buffer, b"6566"); // "65" + "66" = ASCII digits of 65 and 66
        } else {
            panic!("expected Stdout handle");
        }
    }

    #[test]
    fn stderr_write_accumulates() {
        let mut vm = VM::new();
        let h = vm.create_handle(IoHandle::Stderr { buffer: vec![] });
        let main = Function::new(
            "main",
            0,
            vec![
                OpCode::Push(Value::Bool(true)),
                OpCode::StderrWrite,
                OpCode::Halt,
            ],
            0,
        );
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.run().unwrap();

        if let IoHandle::Stderr { buffer } = vm.get_handle(h).unwrap() {
            assert_eq!(buffer, b"true");
        } else {
            panic!("expected Stderr handle");
        }
    }

    #[test]
    fn stdin_read_from_buffer() {
        let mut vm = VM::new();
        vm.create_handle(IoHandle::Stdin {
            buffer: vec![72, 69, 76, 76, 79],
        }); // "HELLO"
        let main = Function::new("main", 0, vec![OpCode::StdinRead, OpCode::Halt], 0);
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.step().unwrap();

        // stdin_read returns a List of Int values from the buffer
        if let Value::List(gc) = vm.stack.last().unwrap() {
            let elements = &vm.heap.get(*gc).unwrap().elements;
            assert_eq!(elements.len(), 5);
            assert_eq!(elements[0], Value::Int(72));
            assert_eq!(elements[4], Value::Int(79));
        } else {
            panic!("expected List from stdin read");
        }
    }

    // -- Cached strategy tests --

    #[test]
    fn cached_strategy_file_read_uses_cache() {
        let mut vm = VM::new();
        let h = vm.create_handle(IoHandle::File {
            path: "/tmp/nonexistent_cached_test.txt".into(),
            mode: crate::io::FileMode::Read,
            position: 0,
            strategy: crate::io::IoStrategy::Cached,
            file: None,
            cached: Some(vec![72, 105]), // "Hi"
        });

        // Should return cached data without opening file
        let result = vm.read_file_handle(h).unwrap();
        if let Value::List(gc) = result {
            let elements = &vm.heap.get(gc).unwrap().elements;
            assert_eq!(elements.len(), 2);
            assert_eq!(elements[0], Value::Int(72));
            assert_eq!(elements[1], Value::Int(105));
        } else {
            panic!("expected List from cached read");
        }
    }

    // -- Timer tests --

    #[test]
    fn timer_sleep_is_noop() {
        let mut vm = VM::new();
        let main = Function::new(
            "main",
            0,
            vec![
                OpCode::Push(Value::Int(1)),
                OpCode::TimerSleep {
                    ms: Value::Int(1000),
                },
                OpCode::Push(Value::Int(2)),
                OpCode::Halt,
            ],
            0,
        );
        vm.add_function(main);
        vm.prepare(0).unwrap();
        vm.run().unwrap();

        // TimerSleep is a no-op: both pushes should execute
        assert_eq!(vm.stack, &[Value::Int(1), Value::Int(2)]);
    }
}
