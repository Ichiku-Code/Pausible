// Snapshot binary format uses u32 for counts; heap / frame sizes in
// practical programs fit comfortably within a 32-bit range.
#![allow(clippy::cast_possible_truncation)]

use crate::io::{HandleId, IoHandle, IoStrategy};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::heap::{Heap, HeapObject};
use crate::value::Value;
use crate::vm::{CallFrame, VM};

// -- header constants --

const MAGIC: [u8; 4] = *b"PAUS";
const VERSION: u32 = 3;

// Value type tags (mirrors serialize.rs wire format).
const TAG_INT: u8 = 0x00;
const TAG_FLOAT: u8 = 0x01;
const TAG_BOOL: u8 = 0x02;
const TAG_NULL: u8 = 0x03;
const TAG_STRING: u8 = 0x04;
const TAG_LIST: u8 = 0x05;

// -- errors --

/// Errors that can occur during snapshot write / read / restore.
#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotError {
    /// File magic does not match `"PAUS"`.
    BadMagic { found: [u8; 4] },
    /// Snapshot version is not supported by this runtime.
    UnsupportedVersion(u32),
    /// Code hash mismatch — bytecode has changed since snapshot was taken.
    CodeMismatch { expected: u64, found: u64 },
    /// File I/O error.
    IoError(String),
    /// Premature end of file.
    UnexpectedEof,
    /// Unknown object tag in heap section.
    UnknownObjectTag(u8),
    /// Heap position refers to an object that was never deserialized.
    InvalidHeapPosition(u32),
}

impl core::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic { found } => write!(f, "bad snapshot magic: {found:02X?}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported snapshot version: {v}"),
            Self::CodeMismatch { expected, found } => {
                write!(
                    f,
                    "code hash mismatch: expected {expected:016X}, found {found:016X}"
                )
            }
            Self::IoError(msg) => write!(f, "snapshot I/O error: {msg}"),
            Self::UnexpectedEof => write!(f, "unexpected end of snapshot data"),
            Self::UnknownObjectTag(b) => write!(f, "unknown heap object tag: {b:#04X}"),
            Self::InvalidHeapPosition(pos) => {
                write!(f, "invalid heap position reference: {pos}")
            }
        }
    }
}

impl core::error::Error for SnapshotError {}
// -- ReconnectStatus / ReconnectReport --

/// Outcome of reconnecting a single I/O handle during resume.
#[derive(Debug, Clone, PartialEq)]
pub enum ReconnectStatus {
    /// Handle reconnected successfully.
    Ok,
    /// Handle reconnected with degraded capability.
    Degraded { reason: String },
    /// Handle could not be reconnected.
    Failed { reason: String },
}

impl ReconnectStatus {
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Collection of per-handle reconnect outcomes.
#[derive(Debug, Clone)]
pub struct ReconnectReport {
    pub entries: Vec<(HandleId, ReconnectStatus)>,
}

impl ReconnectReport {
    /// True if any entry has status `Failed`.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.entries.iter().any(|(_, s)| s.is_failed())
    }
}

// -- Snapshot --

/// A portable, serialized snapshot of the complete VM state.
///
/// Contains the heap objects reachable from roots, all call frames,
/// and the operand stack — everything needed to reconstruct the VM
/// and resume execution from a yield point.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Magic + version + metadata.
    pub header: SnapshotHeader,
    /// Raw bytes of the serialized heap section.
    heap_data: Vec<u8>,
    /// Raw bytes of the serialized frames section.
    frames_data: Vec<u8>,
    /// Raw bytes of the serialized stack section.
    /// Raw bytes of the serialized I/O handles section.
    io_section: Vec<u8>,
    stack_data: Vec<u8>,
    /// Mapping from original Gc index → position in the heap section
    /// (populated during capture; the reverse map pos→Gc is built
    #[allow(dead_code)]
    /// during restore from the heap section itself).
    gc_to_pos: HashMap<usize, u32>,
    /// Serialised task tree state.
    pub task_tree: Vec<TaskSnapshot>,
    /// Raw bytes of the serialised task section.
    pub task_section: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SnapshotHeader {
    pub magic: [u8; 4],
    pub version: u32,
    /// Hash of the serialized function table — must match on restore.
    pub code_hash: u64,
    /// Unix timestamp when the snapshot was created.
    pub timestamp: u64,
    pub heap_count: u32,
    pub frame_count: u32,
    pub stack_count: u32,
    /// Number of serialized I/O handles. Zero for old snapshots.
    pub io_handle_count: u32,
    /// Number of serialized tasks.
    pub task_count: u32,
}
/// Serialised form of a single I/O handle inside a snapshot.
#[derive(Debug, Clone)]
pub struct IoHandleSnapshot {
    pub id: u32,
    /// Kind tag: 0=File, 1=TcpStream, 2=HttpConnection, 3=Timer,
    ///            4=Stdin, 5=Stdout, 6=Stderr.
    pub kind: u8,
    /// Strategy tag: 0=Replay, 1=Seek, 2=Cached.
    pub strategy: u8,
    /// Serialised reconnect parameters (path, addr, url, etc.).
    pub params: Vec<u8>,
    /// Cached data for Ephemeral handles.
    pub cached: Option<Vec<u8>>,
    /// File position for Seekable handles.
    pub position: Option<u64>,
}

/// Serialised form of a single task inside a snapshot.
#[derive(Debug, Clone)]
pub struct TaskSnapshot {
    pub id: u32,
    pub parent: Option<u32>,
    /// 1=Yielded, 2=Completed.
    pub status_kind: u8,
    /// Resume PC for Yielded tasks.
    pub yielded_pc: Option<u32>,
    pub children: Vec<u32>,
    /// Serialised stack values (position-reference format).
    pub stack_data: Vec<u8>,
    /// Serialised frames.
    pub frames_data: Vec<u8>,
    /// Serialised per-task I/O handles.
    pub io_data: Vec<u8>,
    pub io_handle_count: u32,
}

impl Snapshot {
    // -- binary helpers (identical to chunk.rs pattern) --

    fn write_u8(buf: &mut Vec<u8>, b: u8) {
        buf.push(b);
    }

    fn write_u32(buf: &mut Vec<u8>, v: u32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_u64(buf: &mut Vec<u8>, v: u64) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    fn read_u8(data: &[u8], pos: &mut usize) -> Result<u8, SnapshotError> {
        let b = data.get(*pos).ok_or(SnapshotError::UnexpectedEof)?;
        *pos += 1;
        Ok(*b)
    }

    fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, SnapshotError> {
        let end = pos.checked_add(4).ok_or(SnapshotError::UnexpectedEof)?;
        let bytes: [u8; 4] = data
            .get(*pos..end)
            .ok_or(SnapshotError::UnexpectedEof)?
            .try_into()
            .unwrap();
        *pos = end;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(data: &[u8], pos: &mut usize) -> Result<u64, SnapshotError> {
        let end = pos.checked_add(8).ok_or(SnapshotError::UnexpectedEof)?;
        let bytes: [u8; 8] = data
            .get(*pos..end)
            .ok_or(SnapshotError::UnexpectedEof)?
            .try_into()
            .unwrap();
        *pos = end;
        Ok(u64::from_le_bytes(bytes))
    }

    // -- capture --

    /// Capture the current VM state into a snapshot.
    ///
    /// Side-effect: calls `mark_roots` internally to identify reachable
    /// heap objects. Stack and frame state are unchanged.
    ///
    /// # Panics
    ///
    /// Panics if a `Gc` handle points to a freed slot — this indicates
    /// a GC bug and should never happen with a healthy VM.
    pub fn capture(vm: &mut VM, code_hash: u64) -> Self {
        // 1. Mark all reachable objects from roots (including task registry).
        vm.mark_all_task_roots();

        // 2. Serialize reachable heap objects, building gc→position map.
        let mut heap_buf: Vec<u8> = Vec::new();
        let mut gc_to_pos: HashMap<usize, u32> = HashMap::new();
        let mut heap_count: u32 = 0;

        for idx in 0..vm.heap.capacity() {
            if vm.heap.is_marked(idx) {
                gc_to_pos.insert(idx, heap_count);
                heap_count += 1;

                let obj = vm.heap.get_object(idx).expect("idx within capacity");
                Self::serialize_heap_object(&mut heap_buf, obj, &gc_to_pos, &vm.heap);
            }
        }

        // 3. Serialize frames (locals reference heap by position).
        let mut frames_buf: Vec<u8> = Vec::new();
        Self::write_u32(&mut frames_buf, vm.frames.len() as u32);
        for frame in &vm.frames {
            Self::serialize_frame(&mut frames_buf, frame, &gc_to_pos, &vm.heap);
        }

        // 4. Serialize operand stack.
        let mut stack_buf: Vec<u8> = Vec::new();
        Self::write_u32(&mut stack_buf, vm.stack.len() as u32);
        for val in &vm.stack {
            Self::write_value_ref(&mut stack_buf, val, &gc_to_pos, &vm.heap);
        }

        // 4.5. Serialize I/O handles.
        let mut io_buf: Vec<u8> = Vec::new();
        let io_count = vm.handles.len() as u32;
        Self::write_u32(&mut io_buf, io_count);
        for (&id, handle) in &vm.handles {
            Self::serialize_io_handle_snapshot(&mut io_buf, id, handle);
        }

        // 4.6. Serialize task tree (non-Running tasks from registry).
        let mut task_buf: Vec<u8> = Vec::new();
        let tasks: Vec<_> = vm
            .task_registry
            .values()
            .filter(|t| !t.status.is_running())
            .collect();
        let task_count = u32::try_from(tasks.len()).unwrap_or(0);
        Self::write_u32(&mut task_buf, task_count);
        for task in &tasks {
            Self::serialize_task_snapshot(&mut task_buf, task, &gc_to_pos, &vm.heap);
        }

        // 5. Assemble header.
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let header = SnapshotHeader {
            magic: MAGIC,
            version: VERSION,
            code_hash,
            timestamp,
            heap_count,
            frame_count: vm.frames.len() as u32,
            stack_count: vm.stack.len() as u32,
            io_handle_count: io_count,
            task_count,
        };

        Snapshot {
            header,
            heap_data: heap_buf,
            frames_data: frames_buf,
            stack_data: stack_buf,
            io_section: io_buf,
            task_tree: Vec::new(),
            task_section: task_buf,
            gc_to_pos,
        }
    }

    // -- file I/O --

    /// Write the snapshot to a file at `path`.
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError::IoError` if the file cannot be written.
    pub fn write_to_file(&self, path: &str) -> Result<(), SnapshotError> {
        let mut buf: Vec<u8> = Vec::new();

        // Header
        buf.extend_from_slice(&self.header.magic);
        Self::write_u32(&mut buf, self.header.version);
        Self::write_u64(&mut buf, self.header.code_hash);
        Self::write_u64(&mut buf, self.header.timestamp);
        Self::write_u32(&mut buf, self.header.heap_count);
        Self::write_u32(&mut buf, self.header.frame_count);
        Self::write_u32(&mut buf, self.header.stack_count);
        Self::write_u32(&mut buf, self.header.io_handle_count);
        Self::write_u32(&mut buf, self.header.task_count);

        // Sections
        buf.extend_from_slice(&self.heap_data);
        buf.extend_from_slice(&self.frames_data);
        buf.extend_from_slice(&self.stack_data);
        buf.extend_from_slice(&self.io_section);
        buf.extend_from_slice(&self.task_section);

        std::fs::write(path, &buf).map_err(|e| SnapshotError::IoError(e.to_string()))
    }

    /// Read a snapshot from a file at `path`.
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError` if the file cannot be read or the data
    /// is malformed.
    pub fn read_from_file(path: &str) -> Result<Self, SnapshotError> {
        let data = std::fs::read(path).map_err(|e| SnapshotError::IoError(e.to_string()))?;
        Self::read_from_bytes(&data)
    }

    /// Parse a snapshot from in-memory bytes.
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError` if the data is malformed.
    fn read_from_bytes(data: &[u8]) -> Result<Self, SnapshotError> {
        let mut pos: usize = 0;

        // Header
        let magic: [u8; 4] = data
            .get(pos..pos + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or(SnapshotError::UnexpectedEof)?;
        if magic != MAGIC {
            return Err(SnapshotError::BadMagic { found: magic });
        }
        pos += 4;

        let version = Self::read_u32(data, &mut pos)?;
        if version > VERSION {
            return Err(SnapshotError::UnsupportedVersion(version));
        }

        let code_hash = Self::read_u64(data, &mut pos)?;
        let timestamp = Self::read_u64(data, &mut pos)?;
        let heap_count = Self::read_u32(data, &mut pos)?;
        let frame_count = Self::read_u32(data, &mut pos)?;
        let stack_count = Self::read_u32(data, &mut pos)?;
        // Backward compat: v1 snapshots have no io_handle_count field.
        let io_handle_count = if version >= 2 {
            Self::read_u32(data, &mut pos)?
        } else {
            0
        };

        // Backward compat: v1/v2 snapshots have no task_count field.
        let task_count = if version >= 3 {
            Self::read_u32(data, &mut pos)?
        } else {
            0
        };

        // Read heap section: each object is self-describing.
        let heap_start = pos;
        for _ in 0..heap_count {
            Self::skip_heap_object(data, &mut pos)?;
        }
        let heap_data = data[heap_start..pos].to_vec();

        // Read frames section: count(u32) then frame entries.
        let frames_start = pos;
        let _frame_count_data = Self::read_u32(data, &mut pos)?;
        for _ in 0..frame_count {
            Self::skip_frame_entry(data, &mut pos)?;
        }
        let frames_data = data[frames_start..pos].to_vec();

        // Read stack section: count(u32) then values.
        let stack_start = pos;
        let _stack_count_data = Self::read_u32(data, &mut pos)?;
        for _ in 0..stack_count {
            Self::skip_value(data, &mut pos)?;
        }
        let stack_data = data[stack_start..pos].to_vec();

        // Read I/O section: count(u32) then io entries.
        let io_section = if version >= 2 {
            let io_start = pos;
            let _io_count_data = Self::read_u32(data, &mut pos)?;
            for _ in 0..io_handle_count {
                Self::skip_io_entry(data, &mut pos)?;
            }
            data[io_start..pos].to_vec()
        } else {
            Vec::new()
        };

        // Read task section: count(u32) then task entries.
        let task_section = if task_count > 0 {
            let task_start = pos;
            let _task_count_data = Self::read_u32(data, &mut pos)?;
            for _ in 0..task_count {
                Self::skip_task_entry(data, &mut pos)?;
            }
            data[task_start..pos].to_vec()
        } else {
            Vec::new()
        };

        let header = SnapshotHeader {
            magic,
            version,
            code_hash,
            timestamp,
            heap_count,
            frame_count,
            stack_count,
            io_handle_count,
            task_count,
        };

        Ok(Snapshot {
            header,
            heap_data,
            frames_data,
            stack_data,
            io_section,
            task_tree: Vec::new(),
            task_section,
            gc_to_pos: HashMap::new(),
        })
    }

    // -- skip helpers for section boundary computation --

    /// Skip past a snapshot value (position-reference format) without
    /// allocating. Used to find section boundaries during deserialization.
    fn skip_value(data: &[u8], pos: &mut usize) -> Result<(), SnapshotError> {
        let tag = Self::read_u8(data, pos)?;
        match tag {
            TAG_INT | TAG_FLOAT => *pos += 8,
            TAG_BOOL => *pos += 1,
            TAG_NULL => {}
            TAG_STRING | TAG_LIST => {
                *pos += 4;
            }
            _ => return Err(SnapshotError::UnknownObjectTag(tag)),
        }
        Ok(())
    }

    /// Skip past one heap object (inline format with full content).
    fn skip_heap_object(data: &[u8], pos: &mut usize) -> Result<(), SnapshotError> {
        let tag = Self::read_u8(data, pos)?;
        match tag {
            TAG_STRING => {
                let len = Self::read_u32(data, pos)? as usize;
                *pos = pos.checked_add(len).ok_or(SnapshotError::UnexpectedEof)?;
            }
            TAG_LIST => {
                let count = Self::read_u32(data, pos)? as usize;
                for _ in 0..count {
                    Self::skip_value(data, pos)?;
                }
            }
            _ => return Err(SnapshotError::UnknownObjectTag(tag)),
        }
        Ok(())
    }

    /// Skip past one frame entry: function(u32) | ip(u32) |
    /// `locals_count(u32)` | locals values.
    fn skip_frame_entry(data: &[u8], pos: &mut usize) -> Result<(), SnapshotError> {
        // function + ip + locals_count = 3 x u32
        *pos = pos.checked_add(12).ok_or(SnapshotError::UnexpectedEof)?;
        // Re-read locals_count from the correct offset.
        let locals_offset = pos.checked_sub(4).ok_or(SnapshotError::UnexpectedEof)?;
        let locals_count = u32::from_le_bytes(
            data.get(locals_offset..*pos)
                .ok_or(SnapshotError::UnexpectedEof)?
                .try_into()
                .unwrap(),
        ) as usize;
        for _ in 0..locals_count {
            Self::skip_value(data, pos)?;
        }
        Ok(())
    }

    /// Skip past one I/O handle entry.
    fn skip_io_entry(data: &[u8], pos: &mut usize) -> Result<(), SnapshotError> {
        // id(u32) + kind(u8) + strategy(u8) + params_len(u32)
        *pos = pos.checked_add(10).ok_or(SnapshotError::UnexpectedEof)?;
        let params_offset = pos.checked_sub(4).ok_or(SnapshotError::UnexpectedEof)?;
        let params_len = u32::from_le_bytes(
            data.get(params_offset..params_offset + 4)
                .ok_or(SnapshotError::UnexpectedEof)?
                .try_into()
                .unwrap(),
        ) as usize;
        *pos = pos
            .checked_add(params_len)
            .ok_or(SnapshotError::UnexpectedEof)?;
        // cached_len(u32)
        let cached_len = Self::read_u32(data, pos)? as usize;
        // Sentinel 0xFFFF_FFFF means None (no cached data).
        if cached_len != 0xFFFF_FFFF {
            *pos = pos
                .checked_add(cached_len)
                .ok_or(SnapshotError::UnexpectedEof)?;
        }
        // position(u64) — always present, 0xFFFF_FFFF_FFFF_FFFF means None
        *pos = pos.checked_add(8).ok_or(SnapshotError::UnexpectedEof)?;
        Ok(())
    }

    /// Skip past one task snapshot entry.
    fn skip_task_entry(data: &[u8], pos: &mut usize) -> Result<(), SnapshotError> {
        // id(u32)
        *pos = pos.checked_add(4).ok_or(SnapshotError::UnexpectedEof)?;
        // parent_flag(u8)
        let parent_flag = Self::read_u8(data, pos)?;
        if parent_flag == 1 {
            *pos = pos.checked_add(4).ok_or(SnapshotError::UnexpectedEof)?;
        }
        // status_kind(u8)
        let status_kind = Self::read_u8(data, pos)?;
        if status_kind == 1 {
            *pos = pos.checked_add(4).ok_or(SnapshotError::UnexpectedEof)?;
        }
        // children_count(u32) + children ids
        let children_count = Self::read_u32(data, pos)? as usize;
        *pos = pos
            .checked_add(children_count * 4)
            .ok_or(SnapshotError::UnexpectedEof)?;
        // stack_count(u32) + values
        let stack_count = Self::read_u32(data, pos)? as usize;
        for _ in 0..stack_count {
            Self::skip_value(data, pos)?;
        }
        // frame_count(u32) + frames
        let frame_count = Self::read_u32(data, pos)? as usize;
        for _ in 0..frame_count {
            Self::skip_frame_entry(data, pos)?;
        }
        // io_handle_count(u32) + io entries
        let io_count = Self::read_u32(data, pos)? as usize;
        for _ in 0..io_count {
            Self::skip_io_entry(data, pos)?;
        }
        Ok(())
    }

    /// Serialize an I/O handle into a snapshot entry.
    #[allow(clippy::too_many_lines)]
    fn serialize_io_handle_snapshot(buf: &mut Vec<u8>, id: HandleId, handle: &IoHandle) {
        use IoHandle::{File, HttpConnection, Stderr, Stdin, Stdout, TcpStream, Timer};
        Self::write_u32(buf, id.0);
        let (kind, strategy, params, cached, position) = match handle {
            File {
                path,
                mode,
                position,
                strategy,
                cached,
                ..
            } => {
                let mut p = Vec::new();
                let path_bytes = path.as_bytes();
                Self::write_u32(&mut p, path_bytes.len() as u32);
                p.extend_from_slice(path_bytes);
                p.push(*mode as u8);
                (0u8, *strategy as u8, p, cached.clone(), Some(*position))
            }
            TcpStream {
                addr,
                strategy,
                last_request,
                last_response,
                ..
            } => {
                let mut p = Vec::new();
                let addr_bytes = addr.as_bytes();
                Self::write_u32(&mut p, addr_bytes.len() as u32);
                p.extend_from_slice(addr_bytes);
                // last_request
                if let Some(req) = last_request {
                    Self::write_u32(&mut p, req.len() as u32);
                    p.extend_from_slice(req);
                } else {
                    Self::write_u32(&mut p, 0xFFFF_FFFF);
                }
                // last_response
                if let Some(resp) = last_response {
                    Self::write_u32(&mut p, resp.len() as u32);
                    p.extend_from_slice(resp);
                } else {
                    Self::write_u32(&mut p, 0xFFFF_FFFF);
                }
                (1u8, *strategy as u8, p, None, None)
            }
            HttpConnection {
                url,
                method,
                body,
                last_response,
                strategy,
                ..
            } => {
                let mut p = Vec::new();
                let url_bytes = url.as_bytes();
                Self::write_u32(&mut p, url_bytes.len() as u32);
                p.extend_from_slice(url_bytes);
                p.push(*method as u8);
                if let Some(b) = body {
                    Self::write_u32(&mut p, b.len() as u32);
                    p.extend_from_slice(b);
                } else {
                    Self::write_u32(&mut p, 0xFFFF_FFFF);
                }
                if let Some(resp) = last_response {
                    Self::write_u32(&mut p, resp.len() as u32);
                    p.extend_from_slice(resp);
                } else {
                    Self::write_u32(&mut p, 0xFFFF_FFFF);
                }
                (2u8, *strategy as u8, p, None, None)
            }
            Timer { ms, strategy, .. } => {
                let mut p = Vec::new();
                p.extend_from_slice(&ms.to_le_bytes());
                (3u8, *strategy as u8, p, None, None)
            }
            Stdin { buffer } => (
                4u8,
                IoStrategy::Cached as u8,
                Vec::new(),
                Some(buffer.clone()),
                None,
            ),
            Stdout { buffer } => (
                5u8,
                IoStrategy::Cached as u8,
                Vec::new(),
                Some(buffer.clone()),
                None,
            ),
            Stderr { buffer } => (
                6u8,
                IoStrategy::Cached as u8,
                Vec::new(),
                Some(buffer.clone()),
                None,
            ),
        };
        Self::write_u8(buf, kind);
        Self::write_u8(buf, strategy);
        Self::write_u32(buf, params.len() as u32);
        buf.extend_from_slice(&params);
        // cached
        if let Some(c) = &cached {
            Self::write_u32(buf, c.len() as u32);
            buf.extend_from_slice(c);
        } else {
            Self::write_u32(buf, 0xFFFF_FFFF);
        }
        // position — sentinel for None
        if let Some(pos) = position {
            buf.extend_from_slice(&pos.to_le_bytes());
        } else {
            buf.extend_from_slice(&0xFFFF_FFFF_FFFF_FFFFu64.to_le_bytes());
        }
    }

    /// Serialize a task snapshot entry (metadata + stack + frames + I/O).
    fn serialize_task_snapshot(
        buf: &mut Vec<u8>,
        task: &crate::task::Task,
        gc_to_pos: &HashMap<usize, u32>,
        heap: &Heap,
    ) {
        use crate::task::TaskStatus;
        Self::write_u32(buf, task.id.0 as u32);
        // parent
        if let Some(parent) = task.parent {
            Self::write_u8(buf, 1);
            Self::write_u32(buf, parent.0 as u32);
        } else {
            Self::write_u8(buf, 0);
        }
        // status
        match &task.status {
            TaskStatus::Yielded(pc) => {
                Self::write_u8(buf, 1);
                Self::write_u32(buf, *pc as u32);
            }
            TaskStatus::Completed => {
                Self::write_u8(buf, 2);
            }
            TaskStatus::Running => unreachable!("Running tasks are filtered"),
        }
        // children
        Self::write_u32(buf, task.children.len() as u32);
        for child in &task.children {
            Self::write_u32(buf, child.0 as u32);
        }
        // stack
        Self::write_u32(buf, task.stack.len() as u32);
        for val in &task.stack {
            Self::write_value_ref(buf, val, gc_to_pos, heap);
        }
        // frames
        Self::write_u32(buf, task.frames.len() as u32);
        for frame in &task.frames {
            Self::serialize_frame(buf, frame, gc_to_pos, heap);
        }
        // I/O handles
        let io_count = task.io_handles.len() as u32;
        Self::write_u32(buf, io_count);
        for (&id, handle) in &task.io_handles {
            Self::serialize_io_handle_snapshot(buf, id, handle);
        }
    }

    /// Deserialize a single I/O handle snapshot from bytes at the given position.
    fn deserialize_io_handle_snapshot(
        data: &[u8],
        pos: &mut usize,
    ) -> Result<IoHandleSnapshot, SnapshotError> {
        let id = Self::read_u32(data, pos)?;
        let kind = Self::read_u8(data, pos)?;
        let strategy = Self::read_u8(data, pos)?;
        let params_len = Self::read_u32(data, pos)? as usize;
        let params_end = pos
            .checked_add(params_len)
            .ok_or(SnapshotError::UnexpectedEof)?;
        let params = data
            .get(*pos..params_end)
            .ok_or(SnapshotError::UnexpectedEof)?
            .to_vec();
        *pos = params_end;

        let cached_len = Self::read_u32(data, pos)? as usize;
        let cached = if cached_len == 0xFFFF_FFFF {
            None
        } else {
            let end = pos
                .checked_add(cached_len)
                .ok_or(SnapshotError::UnexpectedEof)?;
            let bytes = data
                .get(*pos..end)
                .ok_or(SnapshotError::UnexpectedEof)?
                .to_vec();
            *pos = end;
            Some(bytes)
        };

        let position_raw = Self::read_u64(data, pos)?;
        let position = if position_raw == 0xFFFF_FFFF_FFFF_FFFF {
            None
        } else {
            Some(position_raw)
        };

        Ok(IoHandleSnapshot {
            id,
            kind,
            strategy,
            params,
            cached,
            position,
        })
    }

    /// Restore I/O handles from the snapshot into the VM, attempting
    /// reconnection for each handle.
    ///
    /// Returns a `ReconnectReport` with the outcome for every handle.
    #[must_use]
    pub fn restore_io_handles(&self, vm: &mut VM) -> ReconnectReport {
        use crate::io::IoStrategy;

        let mut report = ReconnectReport {
            entries: Vec::new(),
        };

        if self.header.io_handle_count == 0 {
            return report;
        }

        let mut pos: usize = 0;
        let Ok(io_count) = Self::read_u32(&self.io_section, &mut pos) else {
            return report;
        };

        for _ in 0..io_count {
            let Ok(snap) = Self::deserialize_io_handle_snapshot(&self.io_section, &mut pos) else {
                continue;
            };
            let hid = HandleId(snap.id);
            let strategy = match snap.strategy {
                0 => IoStrategy::Replay,
                1 => IoStrategy::Seek,
                _ => IoStrategy::Cached,
            };

            let (handle, status) = Self::reconnect_handle(&snap, strategy);
            vm.register_handle(hid, handle);
            report.entries.push((hid, status));
        }

        report
    }

    /// Attempt to re-create and reconnect a single I/O handle from its
    /// serialised snapshot.
    #[allow(clippy::too_many_lines)]
    fn reconnect_handle(
        snap: &IoHandleSnapshot,
        strategy: IoStrategy,
    ) -> (IoHandle, ReconnectStatus) {
        use crate::io::{FileMode, HttpMethod, IoHandle};

        match snap.kind {
            0 => {
                // File
                let mut ppos: usize = 0;
                let path_len = Self::read_u32_or(&snap.params, &mut ppos, 0) as usize;
                let path = String::from_utf8_lossy(
                    snap.params
                        .get(ppos..ppos.saturating_add(path_len))
                        .unwrap_or(b""),
                )
                .into_owned();
                ppos = ppos.saturating_add(path_len);
                let mode = if ppos < snap.params.len() {
                    match snap.params[ppos] {
                        0 => FileMode::Read,
                        1 => FileMode::Write,
                        _ => FileMode::Append,
                    }
                } else {
                    FileMode::Read
                };
                let position = snap.position.unwrap_or(0);

                match std::fs::File::open(&path) {
                    Ok(mut f) => {
                        use std::io::Seek;
                        if let Err(e) = f.seek(std::io::SeekFrom::Start(position)) {
                            (
                                IoHandle::File {
                                    path,
                                    mode,
                                    position: 0,
                                    strategy,
                                    file: None,
                                    cached: snap.cached.clone(),
                                },
                                ReconnectStatus::Degraded {
                                    reason: format!("seek failed: {e}"),
                                },
                            )
                        } else {
                            (
                                IoHandle::File {
                                    path,
                                    mode,
                                    position,
                                    strategy,
                                    file: Some(f),
                                    cached: snap.cached.clone(),
                                },
                                ReconnectStatus::Ok,
                            )
                        }
                    }
                    Err(e) => (
                        IoHandle::File {
                            path,
                            mode,
                            position,
                            strategy,
                            file: None,
                            cached: snap.cached.clone(),
                        },
                        ReconnectStatus::Failed {
                            reason: format!("file open failed: {e}"),
                        },
                    ),
                }
            }
            1 => {
                // TcpStream
                let mut tpos: usize = 0;
                let addr_len = Self::read_u32_or(&snap.params, &mut tpos, 0) as usize;
                let addr = String::from_utf8_lossy(
                    snap.params
                        .get(tpos..tpos.saturating_add(addr_len))
                        .unwrap_or(b""),
                )
                .into_owned();
                tpos = tpos.saturating_add(addr_len);
                let _ = tpos;

                match std::net::TcpStream::connect(&addr) {
                    Ok(stream) => (
                        IoHandle::TcpStream {
                            addr,
                            strategy,
                            stream: Some(stream),
                            last_request: None,
                            last_response: None,
                        },
                        ReconnectStatus::Ok,
                    ),
                    Err(e) => (
                        IoHandle::TcpStream {
                            addr,
                            strategy,
                            stream: None,
                            last_request: None,
                            last_response: None,
                        },
                        ReconnectStatus::Failed {
                            reason: format!("TCP connect failed: {e}"),
                        },
                    ),
                }
            }
            2 => {
                // HttpConnection — replay the original request and compare.
                let mut hpos: usize = 0;
                let url_len = Self::read_u32_or(&snap.params, &mut hpos, 0) as usize;
                let url = String::from_utf8_lossy(
                    snap.params
                        .get(hpos..hpos.saturating_add(url_len))
                        .unwrap_or(b""),
                )
                .into_owned();
                hpos = hpos.saturating_add(url_len);
                let method = if hpos < snap.params.len() {
                    match snap.params[hpos] {
                        1 => HttpMethod::Post,
                        _ => HttpMethod::Get,
                    }
                } else {
                    HttpMethod::Get
                };
                hpos = hpos.saturating_add(1);

                // Parse body from params (None sentinel = 0xFFFF_FFFF).
                let body_len = Self::read_u32_or(&snap.params, &mut hpos, 0xFFFF_FFFF) as usize;
                let body = if body_len == 0xFFFF_FFFF_usize {
                    None
                } else {
                    let end = hpos.saturating_add(body_len);
                    let bytes = snap
                        .params
                        .get(hpos..end.min(snap.params.len()))
                        .unwrap_or(b"")
                        .to_vec();
                    hpos = end;
                    Some(bytes)
                };

                // Parse old last_response from params (None sentinel = 0xFFFF_FFFF).
                let old_resp_len = Self::read_u32_or(&snap.params, &mut hpos, 0xFFFF_FFFF) as usize;
                let old_response = if old_resp_len == 0xFFFF_FFFF_usize {
                    None
                } else {
                    let end = hpos.saturating_add(old_resp_len);
                    let bytes = snap
                        .params
                        .get(hpos..end.min(snap.params.len()))
                        .unwrap_or(b"")
                        .to_vec();
                    let _ = hpos;
                    Some(bytes)
                };

                // Replay the HTTP request.
                let result: Result<Vec<u8>, String> = (|| {
                    let resp = match method {
                        HttpMethod::Post => {
                            let agent = ureq::agent();
                            agent
                                .post(&url)
                                .set("Content-Type", "application/octet-stream")
                                .send_bytes(body.as_deref().unwrap_or(&[]))
                                .map_err(|e| e.to_string())?
                        }
                        HttpMethod::Get => ureq::get(&url).call().map_err(|e| e.to_string())?,
                    };

                    let mut new_body = Vec::new();
                    resp.into_reader()
                        .read_to_end(&mut new_body)
                        .map_err(|e| e.to_string())?;
                    Ok(new_body)
                })();

                match result {
                    Ok(new_response) => {
                        let status = match &old_response {
                            Some(old) if old != &new_response => ReconnectStatus::Degraded {
                                reason: "DataDiverged: response differs from snapshot".into(),
                            },
                            _ => ReconnectStatus::Ok,
                        };
                        (
                            IoHandle::HttpConnection {
                                url,
                                method,
                                body,
                                last_response: Some(new_response),
                                strategy,
                            },
                            status,
                        )
                    }
                    Err(e) => (
                        IoHandle::HttpConnection {
                            url,
                            method,
                            body,
                            last_response: old_response,
                            strategy,
                        },
                        ReconnectStatus::Failed {
                            reason: format!("HTTP replay failed: {e}"),
                        },
                    ),
                }
            }
            3 => {
                // Timer — always ok, just restore ms
                let ms = if snap.params.len() >= 8 {
                    u64::from_le_bytes(snap.params[..8].try_into().unwrap_or([0u8; 8]))
                } else {
                    0
                };
                (IoHandle::Timer { ms, strategy }, ReconnectStatus::Ok)
            }
            4 => {
                // Stdin — always ok, restore cached buffer
                (
                    IoHandle::Stdin {
                        buffer: snap.cached.clone().unwrap_or_default(),
                    },
                    ReconnectStatus::Ok,
                )
            }
            5 => {
                // Stdout — always ok
                (
                    IoHandle::Stdout {
                        buffer: snap.cached.clone().unwrap_or_default(),
                    },
                    ReconnectStatus::Ok,
                )
            }
            6 => {
                // Stderr — always ok
                (
                    IoHandle::Stderr {
                        buffer: snap.cached.clone().unwrap_or_default(),
                    },
                    ReconnectStatus::Ok,
                )
            }
            _ => (
                IoHandle::Stdin { buffer: Vec::new() },
                ReconnectStatus::Failed {
                    reason: format!("unknown handle kind: {}", snap.kind),
                },
            ),
        }
    }

    fn read_u32_or(data: &[u8], pos: &mut usize, default: u32) -> u32 {
        if *pos + 4 > data.len() {
            return default;
        }
        let val = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap_or([0u8; 4]));
        *pos += 4;
        val
    }

    // -- restore --

    /// Restore VM state from this snapshot.
    ///
    /// Verifies the code hash against the provided value.
    ///
    /// **I/O handles:** This method does **not** restore I/O handles; callers
    /// must also invoke [`restore_io_handles`] after this to reconnect file,
    /// socket, and other I/O state.  Skipping that step will leave the VM
    /// with whatever handles happened to be present before the restore, and
    /// the restored program may see incorrect I/O results.
    /// On success the VM's heap, frames, and stack are replaced with
    /// the reconstructed state.
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError::CodeMismatch` if the code hashes differ.
    pub fn restore_into(&self, vm: &mut VM, code_hash: u64) -> Result<(), SnapshotError> {
        if code_hash != self.header.code_hash {
            return Err(SnapshotError::CodeMismatch {
                expected: self.header.code_hash,
                found: code_hash,
            });
        }

        // 1. Rebuild heap: read objects, build pos→Gc map.
        let mut heap = Heap::new();
        let mut pos_to_gc: HashMap<u32, usize> = HashMap::new();
        let mut hpos: usize = 0;

        for pos_idx in 0..self.header.heap_count {
            let tag = Self::read_u8(&self.heap_data, &mut hpos)?;
            match tag {
                TAG_STRING => {
                    let len = Self::read_u32(&self.heap_data, &mut hpos)? as usize;
                    let end = hpos.checked_add(len).ok_or(SnapshotError::UnexpectedEof)?;
                    let bytes = self
                        .heap_data
                        .get(hpos..end)
                        .ok_or(SnapshotError::UnexpectedEof)?;
                    hpos = end;
                    let s = String::from_utf8_lossy(bytes).into_owned();
                    let gc = heap.alloc_string(s);
                    pos_to_gc.insert(pos_idx, gc.index);
                }
                TAG_LIST => {
                    let count = Self::read_u32(&self.heap_data, &mut hpos)? as usize;
                    let mut elements = Vec::with_capacity(count);
                    for _ in 0..count {
                        elements.push(Self::read_value_ref(
                            &self.heap_data,
                            &mut hpos,
                            &pos_to_gc,
                            &mut heap,
                        )?);
                    }
                    let gc = heap.alloc_list(elements);
                    pos_to_gc.insert(pos_idx, gc.index);
                }
                _ => return Err(SnapshotError::UnknownObjectTag(tag)),
            }
        }

        // 2. Rebuild frames.
        let mut fpos: usize = 0;
        let frame_count = Self::read_u32(&self.frames_data, &mut fpos)?;
        let mut frames: Vec<CallFrame> = Vec::with_capacity(frame_count as usize);

        for _ in 0..frame_count {
            let function = Self::read_u32(&self.frames_data, &mut fpos)? as usize;
            let ip = Self::read_u32(&self.frames_data, &mut fpos)? as usize;
            let locals_count = Self::read_u32(&self.frames_data, &mut fpos)? as usize;
            let mut locals = Vec::with_capacity(locals_count);
            for _ in 0..locals_count {
                locals.push(Self::read_value_ref(
                    &self.frames_data,
                    &mut fpos,
                    &pos_to_gc,
                    &mut heap,
                )?);
            }
            frames.push(CallFrame::new(function, locals));
            // Restore the IP (CallFrame::new sets it to 0).
            if let Some(frame) = frames.last_mut() {
                frame.ip = ip;
            }
        }

        // 3. Rebuild stack.
        let mut spos: usize = 0;
        let stack_count = Self::read_u32(&self.stack_data, &mut spos)?;
        let mut stack: Vec<Value> = Vec::with_capacity(stack_count as usize);
        for _ in 0..stack_count {
            stack.push(Self::read_value_ref(
                &self.stack_data,
                &mut spos,
                &pos_to_gc,
                &mut heap,
            )?);
        }

        // 4. Replace VM state.
        vm.heap = heap;
        vm.frames = frames;
        vm.stack = stack;
        vm.running = true;

        Ok(())
    }

    /// Rebuild the VM task registry from the serialised task section.
    ///
    /// Must be called after `restore_into` so the heap is already populated.
    /// The task section stores heap positions (not Gc indices), so we
    /// rebuild `pos_to_gc` from `heap_data` to translate them.
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError` if the task section data is malformed.
    #[allow(clippy::too_many_lines)]
    pub fn restore_task_tree(&self, vm: &mut VM) -> Result<(), SnapshotError> {
        if self.header.task_count == 0 {
            return Ok(());
        }

        // Rebuild pos→Gc mapping from heap_data.
        // Since restore_into allocates sequentially into a fresh heap,
        // the mapping is identity: pos_to_gc[i] = i.
        let mut pos_to_gc: HashMap<u32, usize> = HashMap::new();
        let mut hscan: usize = 0;
        for pos_idx in 0..self.header.heap_count {
            let tag = Self::read_u8(&self.heap_data, &mut hscan)?;
            match tag {
                TAG_STRING => {
                    let len = Self::read_u32(&self.heap_data, &mut hscan)? as usize;
                    hscan = hscan.checked_add(len).ok_or(SnapshotError::UnexpectedEof)?;
                }
                TAG_LIST => {
                    let count = Self::read_u32(&self.heap_data, &mut hscan)? as usize;
                    for _ in 0..count {
                        Self::skip_value(&self.heap_data, &mut hscan)?;
                    }
                }
                _ => return Err(SnapshotError::UnknownObjectTag(tag)),
            }
            pos_to_gc.insert(pos_idx, pos_idx as usize);
        }

        // Parse task section.
        let mut tpos: usize = 0;
        let tcount = Self::read_u32(&self.task_section, &mut tpos)?;
        let current_id = vm.current_task_id;

        for _ in 0..tcount {
            let id = Self::read_u32(&self.task_section, &mut tpos)?;
            let task_id = crate::task::TaskId(u64::from(id));

            let parent_flag = Self::read_u8(&self.task_section, &mut tpos)?;
            let parent = if parent_flag == 1 {
                Some(crate::task::TaskId(u64::from(Self::read_u32(
                    &self.task_section,
                    &mut tpos,
                )?)))
            } else {
                None
            };

            let status_kind = Self::read_u8(&self.task_section, &mut tpos)?;
            let status = match status_kind {
                1 => {
                    let pc = Self::read_u32(&self.task_section, &mut tpos)? as usize;
                    crate::task::TaskStatus::Yielded(pc)
                }
                _ => crate::task::TaskStatus::Completed,
            };

            let children_count = Self::read_u32(&self.task_section, &mut tpos)? as usize;
            let mut children = Vec::with_capacity(children_count);
            for _ in 0..children_count {
                children.push(crate::task::TaskId(u64::from(Self::read_u32(
                    &self.task_section,
                    &mut tpos,
                )?)));
            }

            // Skip the current task — its state comes from restore_into.
            if task_id == current_id {
                // Skip stack, frames, I/O sections for the current task.
                let scount = Self::read_u32(&self.task_section, &mut tpos)? as usize;
                for _ in 0..scount {
                    Self::skip_value(&self.task_section, &mut tpos)?;
                }
                let fcount = Self::read_u32(&self.task_section, &mut tpos)? as usize;
                for _ in 0..fcount {
                    Self::skip_frame_entry(&self.task_section, &mut tpos)?;
                }
                let io_count = Self::read_u32(&self.task_section, &mut tpos)? as usize;
                for _ in 0..io_count {
                    Self::skip_io_entry(&self.task_section, &mut tpos)?;
                }
                // Update the current task's children list in the registry.
                if let Some(task) = vm.task_registry.get_mut(&current_id) {
                    task.children = children;
                    task.status = status;
                }
                continue;
            }

            // Restore non-current tasks into the registry.
            let mut task = crate::task::Task::new(task_id, parent);
            task.children = children;
            task.status = status;

            // Restore stack.
            let scount = Self::read_u32(&self.task_section, &mut tpos)? as usize;
            for _ in 0..scount {
                task.stack.push(Self::read_value_ref(
                    &self.task_section,
                    &mut tpos,
                    &pos_to_gc,
                    &mut vm.heap,
                )?);
            }

            // Restore frames.
            let fcount = Self::read_u32(&self.task_section, &mut tpos)? as usize;
            for _ in 0..fcount {
                let function = Self::read_u32(&self.task_section, &mut tpos)? as usize;
                let ip = Self::read_u32(&self.task_section, &mut tpos)? as usize;
                let locals_count = Self::read_u32(&self.task_section, &mut tpos)? as usize;
                let mut locals = Vec::with_capacity(locals_count);
                for _ in 0..locals_count {
                    locals.push(Self::read_value_ref(
                        &self.task_section,
                        &mut tpos,
                        &pos_to_gc,
                        &mut vm.heap,
                    )?);
                }
                let mut frame = crate::vm::CallFrame::new(function, locals);
                frame.ip = ip;
                task.frames.push(frame);
            }

            // Restore I/O handles.
            let io_count = Self::read_u32(&self.task_section, &mut tpos)? as usize;
            for _ in 0..io_count {
                let snap = Self::deserialize_io_handle_snapshot(&self.task_section, &mut tpos)?;
                let hid = crate::io::HandleId(snap.id);
                let (handle, _status) = Self::reconnect_handle(
                    &snap,
                    match snap.strategy {
                        0 => crate::io::IoStrategy::Replay,
                        1 => crate::io::IoStrategy::Seek,
                        _ => crate::io::IoStrategy::Cached,
                    },
                );
                task.io_handles.insert(hid, handle);
            }

            vm.task_registry.insert(task_id, task);
        }

        Ok(())
    }

    // -- internal serialization helpers --

    /// Serialize a single heap object into `buf`.
    fn serialize_heap_object(
        buf: &mut Vec<u8>,
        obj: &HeapObject,
        gc_to_pos: &HashMap<usize, u32>,
        heap: &Heap,
    ) {
        match obj {
            HeapObject::String(s) => {
                Self::write_u8(buf, TAG_STRING);
                Self::write_u32(buf, s.data.len() as u32);
                buf.extend_from_slice(s.data.as_bytes());
            }
            HeapObject::List(list) => {
                Self::write_u8(buf, TAG_LIST);
                Self::write_u32(buf, list.elements.len() as u32);
                for elem in &list.elements {
                    Self::write_value_ref(buf, elem, gc_to_pos, heap);
                }
            }
        }
    }

    /// Serialize a single call frame into `buf`.
    fn serialize_frame(
        buf: &mut Vec<u8>,
        frame: &CallFrame,
        gc_to_pos: &HashMap<usize, u32>,
        heap: &Heap,
    ) {
        Self::write_u32(buf, frame.function as u32);
        Self::write_u32(buf, frame.ip as u32);
        Self::write_u32(buf, frame.locals.len() as u32);
        for val in &frame.locals {
            Self::write_value_ref(buf, val, gc_to_pos, heap);
        }
    }

    /// Write a `Value`, translating heap Gc indices to snapshot
    /// heap-section positions. Primitive values are written inline.
    fn write_value_ref(
        buf: &mut Vec<u8>,
        val: &Value,
        gc_to_pos: &HashMap<usize, u32>,
        _heap: &Heap,
    ) {
        match val {
            Value::Int(v) => {
                Self::write_u8(buf, TAG_INT);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Float(v) => {
                Self::write_u8(buf, TAG_FLOAT);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Value::Bool(v) => {
                Self::write_u8(buf, TAG_BOOL);
                Self::write_u8(buf, u8::from(*v));
            }
            Value::Null => {
                Self::write_u8(buf, TAG_NULL);
            }
            Value::String(gc) => {
                Self::write_u8(buf, TAG_STRING);
                // Translate Gc index → heap position.
                let pos = gc_to_pos
                    .get(&gc.index)
                    .expect("reachable Gc must have a position");
                Self::write_u32(buf, *pos);
            }
            Value::List(gc) => {
                Self::write_u8(buf, TAG_LIST);
                let pos = gc_to_pos
                    .get(&gc.index)
                    .expect("reachable Gc must have a position");
                Self::write_u32(buf, *pos);
            }
        }
    }

    /// Read a value that was written with `write_value_ref` (heap objects
    /// are represented by position references).
    fn read_value_ref(
        data: &[u8],
        pos: &mut usize,
        pos_to_gc: &HashMap<u32, usize>,
        _heap: &mut Heap,
    ) -> Result<Value, SnapshotError> {
        let tag = Self::read_u8(data, pos)?;
        match tag {
            TAG_INT => {
                let end = pos.checked_add(8).ok_or(SnapshotError::UnexpectedEof)?;
                let bytes: [u8; 8] = data
                    .get(*pos..end)
                    .ok_or(SnapshotError::UnexpectedEof)?
                    .try_into()
                    .unwrap();
                *pos = end;
                Ok(Value::Int(i64::from_le_bytes(bytes)))
            }
            TAG_FLOAT => {
                let end = pos.checked_add(8).ok_or(SnapshotError::UnexpectedEof)?;
                let bytes: [u8; 8] = data
                    .get(*pos..end)
                    .ok_or(SnapshotError::UnexpectedEof)?
                    .try_into()
                    .unwrap();
                *pos = end;
                Ok(Value::Float(f64::from_le_bytes(bytes)))
            }
            TAG_BOOL => {
                let b = Self::read_u8(data, pos)?;
                Ok(Value::Bool(b != 0))
            }
            TAG_NULL => Ok(Value::Null),
            TAG_STRING => {
                let heap_pos = Self::read_u32(data, pos)?;
                let gc_index = pos_to_gc
                    .get(&heap_pos)
                    .ok_or(SnapshotError::InvalidHeapPosition(heap_pos))?;
                // The heap already has this string; we just need a Gc handle.
                // Gc is Copy and the index is stable.
                Ok(Value::String(crate::heap::Gc::new(*gc_index)))
            }
            TAG_LIST => {
                let heap_pos = Self::read_u32(data, pos)?;
                let gc_index = pos_to_gc
                    .get(&heap_pos)
                    .ok_or(SnapshotError::InvalidHeapPosition(heap_pos))?;
                Ok(Value::List(crate::heap::Gc::new(*gc_index)))
            }
            _ => Err(SnapshotError::UnknownObjectTag(tag)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::Function;
    use crate::opcode::OpCode;

    fn make_test_vm() -> VM {
        let mut vm = VM::new();

        // main(idx=0): Push(Int(42)); Halt
        let main = Function::new(
            "main",
            0,
            vec![OpCode::Push(Value::Int(42)), OpCode::Halt],
            0,
        );
        vm.add_function(main);
        vm.prepare(0).unwrap();

        // Push a string onto the stack and a list as a local so we have
        // heap objects in the snapshot.
        let s = vm.alloc_string("hello snapshot".into());
        vm.stack.push(Value::String(s));

        let lst = vm.alloc_list(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        vm.stack.push(Value::List(lst));

        vm
    }

    #[test]
    fn snapshot_roundtrip_basic() {
        let mut vm = make_test_vm();
        let code_hash = vm.code_hash();

        let snap = Snapshot::capture(&mut vm, code_hash);
        assert_eq!(snap.header.magic, MAGIC);
        assert_eq!(snap.header.version, VERSION);
        assert!(snap.header.heap_count > 0, "should have heap objects");
        assert!(snap.header.stack_count > 0, "should have stack values");

        // Restore into a fresh VM with the same functions.
        let mut restored = VM::new();
        let main = Function::new(
            "main",
            0,
            vec![OpCode::Push(Value::Int(42)), OpCode::Halt],
            0,
        );
        restored.add_function(main);
        restored.prepare(0).unwrap();

        let ch = restored.code_hash();
        snap.restore_into(&mut restored, ch).unwrap();

        // Check stack contains the same values.
        assert_eq!(restored.stack.len(), vm.stack.len());
        // Stack items should match (heap refs are new Gc handles, but
        // content should be identical).
        assert!(matches!(restored.stack[0], Value::String(_)));
        assert!(matches!(restored.stack[1], Value::List(_)));

        if let Value::String(gc) = &restored.stack[0] {
            assert_eq!(restored.heap.get(*gc).unwrap().data, "hello snapshot");
        }
        if let Value::List(gc) = &restored.stack[1] {
            assert_eq!(
                restored.heap.get(*gc).unwrap().elements,
                vec![Value::Int(1), Value::Int(2), Value::Int(3)]
            );
        }
    }

    #[test]
    fn snapshot_nested_heap_objects() {
        let mut vm = VM::new();
        let inner = vm.alloc_string("nested".into());
        let outer = vm.alloc_list(vec![Value::String(inner), Value::Int(7)]);
        vm.stack.push(Value::List(outer));

        let code_hash = vm.code_hash();
        let snap = Snapshot::capture(&mut vm, code_hash);

        let mut restored = VM::new();
        let ch = restored.code_hash();
        snap.restore_into(&mut restored, ch).unwrap();

        if let Value::List(outer_gc) = &restored.stack[0] {
            let outer_list = restored.heap.get(*outer_gc).unwrap();
            assert_eq!(outer_list.elements.len(), 2);
            if let Value::String(inner_gc) = &outer_list.elements[0] {
                assert_eq!(restored.heap.get(*inner_gc).unwrap().data, "nested");
            } else {
                panic!("expected String at index 0");
            }
            assert_eq!(outer_list.elements[1], Value::Int(7));
        } else {
            panic!("expected List on stack");
        }
    }

    #[test]
    fn snapshot_preserves_call_frames() {
        let mut vm = VM::new();
        // add_one(x): load 0; push 1; add; ret
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
        // main: push 5; call add_one; halt
        let main = Function::new(
            "main",
            0,
            vec![OpCode::Push(Value::Int(5)), OpCode::Call(1), OpCode::Halt],
            0,
        );
        vm.add_function(main);
        vm.add_function(add_one);
        vm.prepare(0).unwrap();

        // Step into the Call instruction — this creates a frame for add_one
        // with ip=0 and locals=[5].
        vm.step().unwrap(); // Push(Int(5))
        vm.step().unwrap(); // Call(1) → creates add_one frame

        let code_hash = vm.code_hash();
        let snap = Snapshot::capture(&mut vm, code_hash);

        // Restore and verify frames.
        let mut restored = VM::new();
        let add_one_r = Function::new(
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
        let main_r = Function::new(
            "main",
            0,
            vec![OpCode::Push(Value::Int(5)), OpCode::Call(1), OpCode::Halt],
            0,
        );
        restored.add_function(main_r);
        restored.add_function(add_one_r);
        restored.prepare(0).unwrap();

        let ch = restored.code_hash();
        snap.restore_into(&mut restored, ch).unwrap();

        assert!(!restored.frames.is_empty());
        // The active frame should be add_one.
        let frame = restored.frames.last().unwrap();
        assert_eq!(frame.function, 1); // add_one is index 1
        assert_eq!(frame.ip, 0); // hasn't executed yet
        assert_eq!(frame.locals[0], Value::Int(5));
    }

    #[test]
    fn snapshot_file_write_read_roundtrip() {
        let mut vm = make_test_vm();
        let code_hash = vm.code_hash();
        let snap = Snapshot::capture(&mut vm, code_hash);

        let path = "/tmp/pausible_test_snapshot.bin";
        snap.write_to_file(path).unwrap();

        let loaded = Snapshot::read_from_file(path).unwrap();
        assert_eq!(loaded.header.magic, MAGIC);
        assert_eq!(loaded.header.code_hash, code_hash);
        assert_eq!(loaded.header.heap_count, snap.header.heap_count);
        assert_eq!(loaded.header.stack_count, snap.header.stack_count);

        // Restore from the loaded snapshot.
        let mut restored = VM::new();
        let main = Function::new(
            "main",
            0,
            vec![OpCode::Push(Value::Int(42)), OpCode::Halt],
            0,
        );
        restored.add_function(main);
        restored.prepare(0).unwrap();

        let ch = restored.code_hash();
        loaded.restore_into(&mut restored, ch).unwrap();
        assert_eq!(restored.stack.len(), vm.stack.len());
    }

    #[test]
    fn code_hash_mismatch_returns_error() {
        let mut vm = make_test_vm();
        let ch = vm.code_hash();
        let snap = Snapshot::capture(&mut vm, ch);

        // Restore with a wrong code hash.
        let mut restored = VM::new();
        let err = snap
            .restore_into(&mut restored, 0xDEAD_BEEF_CAFE_BABE)
            .unwrap_err();
        assert!(matches!(err, SnapshotError::CodeMismatch { .. }));
    }

    #[test]
    fn snapshot_empty_vm() {
        let mut vm = VM::new();
        let main = Function::new("empty", 0, vec![OpCode::Halt], 0);
        vm.add_function(main);
        vm.prepare(0).unwrap();

        let ch = vm.code_hash();
        let snap = Snapshot::capture(&mut vm, ch);
        assert_eq!(snap.header.heap_count, 0);
        assert_eq!(snap.header.stack_count, 0);

        let mut restored = VM::new();
        let main2 = Function::new("empty", 0, vec![OpCode::Halt], 0);
        restored.add_function(main2);
        restored.prepare(0).unwrap();
        let ch = restored.code_hash();
        snap.restore_into(&mut restored, ch).unwrap();

        assert!(restored.stack.is_empty());
    }

    #[test]
    fn bad_magic_error() {
        let data = vec![0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
        let err = Snapshot::read_from_bytes(&data).unwrap_err();
        assert!(matches!(err, SnapshotError::BadMagic { .. }));
    }

    #[test]
    fn unsupported_version_error() {
        let mut data = Vec::new();
        data.extend_from_slice(&MAGIC);
        data.extend_from_slice(&99u32.to_le_bytes()); // version 99
        data.extend_from_slice(&[0u8; 32]); // pad with zeros
        let err = Snapshot::read_from_bytes(&data).unwrap_err();
        assert!(matches!(err, SnapshotError::UnsupportedVersion(99)));
    }

    #[test]
    fn snapshot_error_display() {
        assert_eq!(
            format!("{}", SnapshotError::UnknownObjectTag(0xFF)),
            "unknown heap object tag: 0xFF"
        );
        assert_eq!(
            format!("{}", SnapshotError::InvalidHeapPosition(7)),
            "invalid heap position reference: 7"
        );
    }
}
