// Serialization uses usize→u32 casts for binary encoding; values are
// guaranteed to fit at this layer (code lengths, indices, etc.).
#![allow(clippy::cast_possible_truncation)]

use core::fmt;

use crate::function::Function;
use crate::heap::Heap;
use crate::io::HandleId;
use crate::opcode::OpCode;
use crate::value::Value;

// -- Chunk builder --

/// Bytecode builder with a chainable API.
#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub code: Vec<OpCode>,
}

impl Chunk {
    #[must_use]
    pub fn new() -> Self {
        Self { code: Vec::new() }
    }

    /// Append a raw opcode; returns self for chaining.
    pub fn emit(&mut self, op: OpCode) -> &mut Self {
        self.code.push(op);
        self
    }

    /// Shortcut: `Push(value)`.
    pub fn emit_push(&mut self, v: Value) -> &mut Self {
        self.emit(OpCode::Push(v))
    }

    /// Shortcut: `Jump(offset)`.
    pub fn emit_jump(&mut self, offset: usize) -> &mut Self {
        self.emit(OpCode::Jump(offset))
    }

    /// Shortcut: `JumpIfTrue(offset)`.
    pub fn emit_jump_if_true(&mut self, offset: usize) -> &mut Self {
        self.emit(OpCode::JumpIfTrue(offset))
    }

    /// Shortcut: `JumpIfFalse(offset)`.
    pub fn emit_jump_if_false(&mut self, offset: usize) -> &mut Self {
        self.emit(OpCode::JumpIfFalse(offset))
    }

    /// Shortcut: `Load(index)`.
    pub fn emit_load(&mut self, idx: usize) -> &mut Self {
        self.emit(OpCode::Load(idx))
    }

    /// Shortcut: `Store(index)`.
    pub fn emit_store(&mut self, idx: usize) -> &mut Self {
        self.emit(OpCode::Store(idx))
    }

    /// Shortcut: `Call(index)`.
    pub fn emit_call(&mut self, idx: usize) -> &mut Self {
        self.emit(OpCode::Call(idx))
    }

    /// Returns the current instruction count (useful for jump offsets).
    #[must_use]
    pub fn len(&self) -> usize {
        self.code.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }

    /// Consume self and return the raw opcode vector.
    #[must_use]
    pub fn into_code(self) -> Vec<OpCode> {
        self.code
    }
}

// -- Binary serialization --

/// Magic bytes for the binary module format: "PAUB" (Pausible Bytecode).
const MAGIC: [u8; 4] = *b"PAUB";
const VERSION: u32 = 1;

/// Opcode byte tags (must match layout below).
mod tag {
    pub const HALT: u8 = 0x00;
    pub const PUSH: u8 = 0x01;
    pub const POP: u8 = 0x02;
    pub const DUP: u8 = 0x03;
    pub const ADD: u8 = 0x04;
    pub const SUB: u8 = 0x05;
    pub const MUL: u8 = 0x06;
    pub const DIV: u8 = 0x07;
    pub const MOD: u8 = 0x08;
    pub const NEG: u8 = 0x09;
    pub const EQ: u8 = 0x0A;
    pub const NEQ: u8 = 0x0B;
    pub const LT: u8 = 0x0C;
    pub const GT: u8 = 0x0D;
    pub const LTE: u8 = 0x0E;
    pub const GTE: u8 = 0x0F;
    pub const AND: u8 = 0x10;
    pub const OR: u8 = 0x11;
    pub const NOT: u8 = 0x12;
    pub const JUMP: u8 = 0x13;
    pub const JUMP_IF_TRUE: u8 = 0x14;
    pub const JUMP_IF_FALSE: u8 = 0x15;
    pub const LOAD: u8 = 0x16;
    pub const STORE: u8 = 0x17;
    pub const CALL: u8 = 0x18;
    pub const RETURN: u8 = 0x19;
    pub const YIELD: u8 = 0x1A;
    pub const FILE_OPEN: u8 = 0x1B;
    pub const FILE_READ: u8 = 0x1C;
    pub const FILE_WRITE: u8 = 0x1D;
    pub const FILE_SEEK: u8 = 0x1E;
    pub const FILE_CLOSE: u8 = 0x1F;
    pub const TCP_CONNECT: u8 = 0x20;
    pub const TCP_READ: u8 = 0x21;
    pub const TCP_WRITE: u8 = 0x22;
    pub const TCP_CLOSE: u8 = 0x23;
    pub const HTTP_GET: u8 = 0x24;
    pub const HTTP_POST: u8 = 0x25;
    pub const STDIN_READ: u8 = 0x26;
    pub const STDOUT_WRITE: u8 = 0x27;
    pub const STDERR_WRITE: u8 = 0x28;
    pub const TIMER_SLEEP: u8 = 0x29;
    pub const SPAWN: u8 = 0x2A;
    pub const WAIT_CHILDREN: u8 = 0x2B;
}

/// Value type tags.
mod val_tag {
    pub const INT: u8 = 0x00;
    pub const FLOAT: u8 = 0x01;
    pub const BOOL: u8 = 0x02;
    pub const NULL: u8 = 0x03;
    pub const STRING: u8 = 0x04;
    pub const LIST: u8 = 0x05;
}

/// Serialization / deserialization errors.
#[derive(Debug, Clone, PartialEq)]
pub enum SerError {
    BadMagic { found: [u8; 4] },
    UnsupportedVersion(u32),
    UnexpectedEof,
    UnknownOpcode(u8),
    UnknownValueTag(u8),
    HeapValueWithoutHeap,
}

impl fmt::Display for SerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic { found } => write!(f, "bad magic: {found:02X?}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported version: {v}"),
            Self::UnexpectedEof => write!(f, "unexpected end of data"),
            Self::UnknownOpcode(b) => write!(f, "unknown opcode byte: {b:#04X}"),
            Self::UnknownValueTag(b) => write!(f, "unknown value tag: {b:#04X}"),
            Self::HeapValueWithoutHeap => write!(
                f,
                "cannot serialize/deserialize a heap-backed Value without a Heap context"
            ),
        }
    }
}

impl core::error::Error for SerError {}

// -- Writers --

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_value(buf: &mut Vec<u8>, val: &Value, heap: &Heap) {
    match val {
        Value::Int(v) => {
            buf.push(val_tag::INT);
            buf.extend_from_slice(&v.to_le_bytes());
        }
        Value::Float(v) => {
            buf.push(val_tag::FLOAT);
            buf.extend_from_slice(&v.to_le_bytes());
        }
        Value::Bool(v) => {
            buf.push(val_tag::BOOL);
            buf.push(u8::from(*v));
        }
        Value::Null => {
            buf.push(val_tag::NULL);
        }
        Value::String(gc) => {
            buf.push(val_tag::STRING);
            let s = heap.get(*gc).expect("valid Gc handle");
            write_u32(buf, s.data.len() as u32);
            buf.extend_from_slice(s.data.as_bytes());
        }
        Value::List(gc) => {
            buf.push(val_tag::LIST);
            let list = heap.get(*gc).expect("valid Gc handle");
            write_u32(buf, list.elements.len() as u32);
            for elem in &list.elements {
                write_value(buf, elem, heap);
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn write_opcode(buf: &mut Vec<u8>, op: &OpCode, heap: &Heap) {
    match op {
        OpCode::Push(v) => {
            buf.push(tag::PUSH);
            write_value(buf, v, heap);
        }
        OpCode::Pop => buf.push(tag::POP),
        OpCode::Dup => buf.push(tag::DUP),
        OpCode::Add => buf.push(tag::ADD),
        OpCode::Sub => buf.push(tag::SUB),
        OpCode::Mul => buf.push(tag::MUL),
        OpCode::Div => buf.push(tag::DIV),
        OpCode::Mod => buf.push(tag::MOD),
        OpCode::Neg => buf.push(tag::NEG),
        OpCode::Eq => buf.push(tag::EQ),
        OpCode::Neq => buf.push(tag::NEQ),
        OpCode::Lt => buf.push(tag::LT),
        OpCode::Gt => buf.push(tag::GT),
        OpCode::Lte => buf.push(tag::LTE),
        OpCode::Gte => buf.push(tag::GTE),
        OpCode::And => buf.push(tag::AND),
        OpCode::Or => buf.push(tag::OR),
        OpCode::Not => buf.push(tag::NOT),
        OpCode::Jump(off) => {
            buf.push(tag::JUMP);
            write_u32(buf, *off as u32);
        }
        OpCode::JumpIfTrue(off) => {
            buf.push(tag::JUMP_IF_TRUE);
            write_u32(buf, *off as u32);
        }
        OpCode::JumpIfFalse(off) => {
            buf.push(tag::JUMP_IF_FALSE);
            write_u32(buf, *off as u32);
        }
        OpCode::Load(idx) => {
            buf.push(tag::LOAD);
            write_u32(buf, *idx as u32);
        }
        OpCode::Store(idx) => {
            buf.push(tag::STORE);
            write_u32(buf, *idx as u32);
        }
        OpCode::Call(idx) => {
            buf.push(tag::CALL);
            write_u32(buf, *idx as u32);
        }
        OpCode::Return => buf.push(tag::RETURN),
        OpCode::Yield => buf.push(tag::YIELD),
        OpCode::Halt => buf.push(tag::HALT),
        OpCode::FileOpen { path, mode } => {
            buf.push(tag::FILE_OPEN);
            write_value(buf, path, heap);
            write_value(buf, mode, heap);
        }
        OpCode::FileRead(h) => {
            buf.push(tag::FILE_READ);
            write_u32(buf, h.0);
        }
        OpCode::FileWrite(h) => {
            buf.push(tag::FILE_WRITE);
            write_u32(buf, h.0);
        }
        OpCode::FileSeek { handle, offset } => {
            buf.push(tag::FILE_SEEK);
            write_u32(buf, handle.0);
            write_value(buf, offset, heap);
        }
        OpCode::FileClose(h) => {
            buf.push(tag::FILE_CLOSE);
            write_u32(buf, h.0);
        }
        OpCode::TcpConnect { addr } => {
            buf.push(tag::TCP_CONNECT);
            write_value(buf, addr, heap);
        }
        OpCode::TcpRead(h) => {
            buf.push(tag::TCP_READ);
            write_u32(buf, h.0);
        }
        OpCode::TcpWrite(h) => {
            buf.push(tag::TCP_WRITE);
            write_u32(buf, h.0);
        }
        OpCode::TcpClose(h) => {
            buf.push(tag::TCP_CLOSE);
            write_u32(buf, h.0);
        }
        OpCode::HttpGet { url } => {
            buf.push(tag::HTTP_GET);
            write_value(buf, url, heap);
        }
        OpCode::HttpPost { url, body } => {
            buf.push(tag::HTTP_POST);
            write_value(buf, url, heap);
            write_value(buf, body, heap);
        }
        OpCode::StdinRead => buf.push(tag::STDIN_READ),
        OpCode::StdoutWrite => buf.push(tag::STDOUT_WRITE),
        OpCode::StderrWrite => buf.push(tag::STDERR_WRITE),
        OpCode::TimerSleep { ms } => {
            buf.push(tag::TIMER_SLEEP);
            write_value(buf, ms, heap);
        }
        OpCode::Spawn(func_id) => {
            buf.push(tag::SPAWN);
            write_u32(buf, *func_id as u32);
        }
        OpCode::WaitChildren => buf.push(tag::WAIT_CHILDREN),
    }
}

// -- Readers --

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, SerError> {
    let end = pos.checked_add(4).ok_or(SerError::UnexpectedEof)?;
    let bytes: [u8; 4] = data
        .get(*pos..end)
        .ok_or(SerError::UnexpectedEof)?
        .try_into()
        .unwrap();
    *pos = end;
    Ok(u32::from_le_bytes(bytes))
}

fn read_value(data: &[u8], pos: &mut usize, heap: &mut Heap) -> Result<Value, SerError> {
    let tag = data.get(*pos).ok_or(SerError::UnexpectedEof)?;
    *pos += 1;
    match *tag {
        val_tag::INT => {
            let end = pos.checked_add(8).ok_or(SerError::UnexpectedEof)?;
            let bytes: [u8; 8] = data
                .get(*pos..end)
                .ok_or(SerError::UnexpectedEof)?
                .try_into()
                .unwrap();
            *pos = end;
            Ok(Value::Int(i64::from_le_bytes(bytes)))
        }
        val_tag::FLOAT => {
            let end = pos.checked_add(8).ok_or(SerError::UnexpectedEof)?;
            let bytes: [u8; 8] = data
                .get(*pos..end)
                .ok_or(SerError::UnexpectedEof)?
                .try_into()
                .unwrap();
            *pos = end;
            Ok(Value::Float(f64::from_le_bytes(bytes)))
        }
        val_tag::BOOL => {
            let b = data.get(*pos).ok_or(SerError::UnexpectedEof)?;
            *pos += 1;
            Ok(Value::Bool(*b != 0))
        }
        val_tag::NULL => Ok(Value::Null),
        val_tag::STRING => {
            let len = read_u32(data, pos)? as usize;
            let end = pos.checked_add(len).ok_or(SerError::UnexpectedEof)?;
            let bytes = data.get(*pos..end).ok_or(SerError::UnexpectedEof)?;
            *pos = end;
            let s = String::from_utf8_lossy(bytes).into_owned();
            let gc = heap.alloc_string(s);
            Ok(Value::String(gc))
        }
        val_tag::LIST => {
            let count = read_u32(data, pos)? as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                elements.push(read_value(data, pos, heap)?);
            }
            let gc = heap.alloc_list(elements);
            Ok(Value::List(gc))
        }
        _ => Err(SerError::UnknownValueTag(*tag)),
    }
}

#[allow(clippy::too_many_lines)]
fn read_opcode(data: &[u8], pos: &mut usize, heap: &mut Heap) -> Result<OpCode, SerError> {
    let tag = data.get(*pos).ok_or(SerError::UnexpectedEof)?;
    *pos += 1;
    match *tag {
        tag::HALT => Ok(OpCode::Halt),
        tag::PUSH => {
            let val = read_value(data, pos, heap)?;
            Ok(OpCode::Push(val))
        }
        tag::POP => Ok(OpCode::Pop),
        tag::DUP => Ok(OpCode::Dup),
        tag::ADD => Ok(OpCode::Add),
        tag::SUB => Ok(OpCode::Sub),
        tag::MUL => Ok(OpCode::Mul),
        tag::DIV => Ok(OpCode::Div),
        tag::MOD => Ok(OpCode::Mod),
        tag::NEG => Ok(OpCode::Neg),
        tag::EQ => Ok(OpCode::Eq),
        tag::NEQ => Ok(OpCode::Neq),
        tag::LT => Ok(OpCode::Lt),
        tag::GT => Ok(OpCode::Gt),
        tag::LTE => Ok(OpCode::Lte),
        tag::GTE => Ok(OpCode::Gte),
        tag::AND => Ok(OpCode::And),
        tag::OR => Ok(OpCode::Or),
        tag::NOT => Ok(OpCode::Not),
        tag::JUMP => {
            let off = read_u32(data, pos)? as usize;
            Ok(OpCode::Jump(off))
        }
        tag::JUMP_IF_TRUE => {
            let off = read_u32(data, pos)? as usize;
            Ok(OpCode::JumpIfTrue(off))
        }
        tag::JUMP_IF_FALSE => {
            let off = read_u32(data, pos)? as usize;
            Ok(OpCode::JumpIfFalse(off))
        }
        tag::LOAD => {
            let idx = read_u32(data, pos)? as usize;
            Ok(OpCode::Load(idx))
        }
        tag::STORE => {
            let idx = read_u32(data, pos)? as usize;
            Ok(OpCode::Store(idx))
        }
        tag::CALL => {
            let idx = read_u32(data, pos)? as usize;
            Ok(OpCode::Call(idx))
        }
        tag::RETURN => Ok(OpCode::Return),
        tag::YIELD => Ok(OpCode::Yield),
        tag::FILE_OPEN => {
            let path = read_value(data, pos, heap)?;
            let mode = read_value(data, pos, heap)?;
            Ok(OpCode::FileOpen { path, mode })
        }
        tag::FILE_READ => {
            let h = read_u32(data, pos)?;
            Ok(OpCode::FileRead(HandleId(h)))
        }
        tag::FILE_WRITE => {
            let h = read_u32(data, pos)?;
            Ok(OpCode::FileWrite(HandleId(h)))
        }
        tag::FILE_SEEK => {
            let handle_raw = read_u32(data, pos)?;
            let offset = read_value(data, pos, heap)?;
            Ok(OpCode::FileSeek {
                handle: HandleId(handle_raw),
                offset,
            })
        }
        tag::FILE_CLOSE => {
            let h = read_u32(data, pos)?;
            Ok(OpCode::FileClose(HandleId(h)))
        }
        tag::TCP_CONNECT => {
            let addr = read_value(data, pos, heap)?;
            Ok(OpCode::TcpConnect { addr })
        }
        tag::TCP_READ => {
            let h = read_u32(data, pos)?;
            Ok(OpCode::TcpRead(HandleId(h)))
        }
        tag::TCP_WRITE => {
            let h = read_u32(data, pos)?;
            Ok(OpCode::TcpWrite(HandleId(h)))
        }
        tag::TCP_CLOSE => {
            let h = read_u32(data, pos)?;
            Ok(OpCode::TcpClose(HandleId(h)))
        }
        tag::HTTP_GET => {
            let url = read_value(data, pos, heap)?;
            Ok(OpCode::HttpGet { url })
        }
        tag::HTTP_POST => {
            let url = read_value(data, pos, heap)?;
            let body = read_value(data, pos, heap)?;
            Ok(OpCode::HttpPost { url, body })
        }
        tag::STDIN_READ => Ok(OpCode::StdinRead),
        tag::STDOUT_WRITE => Ok(OpCode::StdoutWrite),
        tag::STDERR_WRITE => Ok(OpCode::StderrWrite),
        tag::TIMER_SLEEP => {
            let ms = read_value(data, pos, heap)?;
            Ok(OpCode::TimerSleep { ms })
        }
        tag::SPAWN => {
            let func_id = read_u32(data, pos)? as usize;
            Ok(OpCode::Spawn(func_id))
        }
        tag::WAIT_CHILDREN => Ok(OpCode::WaitChildren),
        _ => Err(SerError::UnknownOpcode(*tag)),
    }
}

// -- Public API --

/// Serialize a single function to bytes.
#[must_use]
pub fn serialize_function(func: &Function, heap: &Heap) -> Vec<u8> {
    let mut buf = Vec::new();

    write_u32(&mut buf, func.name.len() as u32);
    buf.extend_from_slice(func.name.as_bytes());
    write_u32(&mut buf, func.arity as u32);
    write_u32(&mut buf, func.num_locals as u32);
    write_u32(&mut buf, func.code.len() as u32);
    for op in &func.code {
        write_opcode(&mut buf, op, heap);
    }

    buf
}

/// Deserialize a single function from bytes.
///
/// # Errors
///
/// Returns `SerError` if the data is malformed.
pub fn deserialize_function(data: &[u8], heap: &mut Heap) -> Result<Function, SerError> {
    deserialize_function_counted(data, heap).map(|(f, _)| f)
}

/// Internal: deserialize a function and return bytes consumed.
fn deserialize_function_counted(
    data: &[u8],
    heap: &mut Heap,
) -> Result<(Function, usize), SerError> {
    let mut pos = 0;

    let name_len = read_u32(data, &mut pos)? as usize;
    let end = pos + name_len;
    let name =
        String::from_utf8_lossy(data.get(pos..end).ok_or(SerError::UnexpectedEof)?).into_owned();
    pos += name_len;
    let arity = read_u32(data, &mut pos)? as usize;
    let num_locals = read_u32(data, &mut pos)? as usize;
    let code_len = read_u32(data, &mut pos)? as usize;

    let mut code = Vec::with_capacity(code_len);
    for _ in 0..code_len {
        code.push(read_opcode(data, &mut pos, heap)?);
    }

    Ok((Function::new(&name, arity, code, num_locals), pos))
}

/// Serialize a module (list of functions) to bytes with header.
#[must_use]
pub fn serialize_module(functions: &[Function], heap: &Heap) -> Vec<u8> {
    let mut buf = Vec::new();

    // Header
    buf.extend_from_slice(&MAGIC);
    write_u32(&mut buf, VERSION);
    write_u32(&mut buf, functions.len() as u32);
    let total_instructions: u32 = functions.iter().map(|f| f.code.len() as u32).sum();
    write_u32(&mut buf, total_instructions);

    // Functions
    for func in functions {
        let func_bytes = serialize_function(func, heap);
        buf.extend_from_slice(&func_bytes);
    }

    buf
}

/// Deserialize a module (list of functions) from bytes.
///
/// # Errors
///
/// Returns `SerError` if the data is malformed or version is unsupported.
pub fn deserialize_module(data: &[u8]) -> Result<(Vec<Function>, Heap), SerError> {
    let mut heap = Heap::new();
    let mut pos = 0;

    // Header
    let magic: [u8; 4] = data
        .get(pos..pos + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or(SerError::UnexpectedEof)?;
    if magic != MAGIC {
        return Err(SerError::BadMagic { found: magic });
    }
    pos += 4;

    let version = read_u32(data, &mut pos)?;
    if version != VERSION {
        return Err(SerError::UnsupportedVersion(version));
    }

    let num_functions = read_u32(data, &mut pos)? as usize;
    let _total_instructions = read_u32(data, &mut pos)?;

    let mut functions = Vec::with_capacity(num_functions);
    for _ in 0..num_functions {
        let (func, consumed) = deserialize_function_counted(&data[pos..], &mut heap)?;
        pos += consumed;
        functions.push(func);
    }

    Ok((functions, heap))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::Heap;

    // -- Chunk builder --

    #[test]
    fn chunk_builder_chain() {
        let mut c = Chunk::new();
        c.emit_push(Value::Int(1))
            .emit_push(Value::Int(2))
            .emit(OpCode::Add)
            .emit(OpCode::Halt);
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn chunk_shortcuts() {
        let mut c = Chunk::new();
        c.emit_push(Value::Null)
            .emit_load(0)
            .emit_store(1)
            .emit_call(2)
            .emit_jump(10)
            .emit_jump_if_true(20)
            .emit_jump_if_false(30)
            .emit(OpCode::Return);
        assert_eq!(c.len(), 8);
    }

    #[test]
    fn chunk_into_code() {
        let mut c = Chunk::new();
        c.emit_push(Value::Bool(true)).emit(OpCode::Halt);
        assert_eq!(
            c.into_code(),
            vec![OpCode::Push(Value::Bool(true)), OpCode::Halt]
        );
    }

    // -- Function serialize roundtrip --

    #[test]
    fn function_roundtrip_simple() {
        let func = Function::new(
            "add",
            2,
            vec![
                OpCode::Load(0),
                OpCode::Load(1),
                OpCode::Add,
                OpCode::Return,
            ],
            2,
        );
        let heap = Heap::new();
        let bytes = serialize_function(&func, &heap);
        let mut restored_heap = Heap::new();
        let restored = deserialize_function(&bytes, &mut restored_heap).unwrap();
        assert_eq!(restored.name, "add");
        assert_eq!(restored.arity, 2);
        assert_eq!(restored.num_locals, 2);
        assert_eq!(restored.code, func.code);
    }

    #[test]
    fn function_roundtrip_with_jumps() {
        let func = Function::new(
            "loop",
            0,
            vec![
                OpCode::Push(Value::Int(0)),
                OpCode::Store(0),
                OpCode::Load(0),
                OpCode::Push(Value::Int(10)),
                OpCode::Lt,
                OpCode::JumpIfFalse(20),
                OpCode::Load(0),
                OpCode::Push(Value::Int(1)),
                OpCode::Add,
                OpCode::Store(0),
                OpCode::Jump(2),
                OpCode::Halt,
            ],
            1,
        );
        let heap = Heap::new();
        let bytes = serialize_function(&func, &heap);
        let mut restored_heap = Heap::new();
        let restored = deserialize_function(&bytes, &mut restored_heap).unwrap();
        assert_eq!(restored.code, func.code);
    }

    #[test]
    fn function_roundtrip_all_value_types() {
        let func = Function::new(
            "values",
            0,
            vec![
                OpCode::Push(Value::Int(42)),
                OpCode::Push(Value::Float(1.5)),
                OpCode::Push(Value::Bool(true)),
                OpCode::Push(Value::Bool(false)),
                OpCode::Push(Value::Null),
                OpCode::Halt,
            ],
            0,
        );
        let heap = Heap::new();
        let bytes = serialize_function(&func, &heap);
        let mut restored_heap = Heap::new();
        let restored = deserialize_function(&bytes, &mut restored_heap).unwrap();
        assert_eq!(restored.code, func.code);
    }

    // -- Module serialize roundtrip --

    #[test]
    fn module_roundtrip() {
        let f1 = Function::new("main", 0, vec![OpCode::Call(1), OpCode::Halt], 0);
        let f2 = Function::new(
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
        let functions = vec![f1.clone(), f2.clone()];

        let heap = Heap::new();
        let bytes = serialize_module(&functions, &heap);
        let (restored, _heap) = deserialize_module(&bytes).unwrap();

        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].name, "main");
        assert_eq!(restored[0].code, f1.code);
        assert_eq!(restored[1].code, f2.code);
    }

    #[test]
    fn module_roundtrip_single_function() {
        let func = Function::new("entry", 0, vec![OpCode::Halt], 0);
        let functions = vec![func.clone()];

        let heap = Heap::new();
        let bytes = serialize_module(&functions, &heap);
        let (restored, _heap) = deserialize_module(&bytes).unwrap();

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].name, "entry");
        assert_eq!(restored[0].code, func.code);
    }

    // -- Error cases --

    #[test]
    fn bad_magic_returns_error() {
        let data = vec![0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
        let err = deserialize_module(&data).unwrap_err();
        assert!(matches!(err, SerError::BadMagic { .. }));
    }

    #[test]
    fn unsupported_version_returns_error() {
        let mut data = Vec::new();
        data.extend_from_slice(&MAGIC);
        // version 99
        data.extend_from_slice(&99u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // num functions
        data.extend_from_slice(&0u32.to_le_bytes()); // num instructions
        let err = deserialize_module(&data).unwrap_err();
        assert!(matches!(err, SerError::UnsupportedVersion(99)));
    }

    #[test]
    fn unexpected_eof() {
        // truncated magic
        let err = deserialize_module(&[0x50, 0x41]).unwrap_err();
        assert!(matches!(err, SerError::UnexpectedEof));
    }

    #[test]
    fn unknown_opcode() {
        // After header: function count=1, then function data with bad opcode
        let mut data = Vec::new();
        data.extend_from_slice(&MAGIC);
        data.extend_from_slice(&VERSION.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes()); // 1 function
        data.extend_from_slice(&1u32.to_le_bytes()); // 1 instruction total
        // Function: name_len=0, name="", arity=0, locals=0, code_len=1
        data.extend_from_slice(&0u32.to_le_bytes()); // name_len
        data.extend_from_slice(&0u32.to_le_bytes()); // arity
        data.extend_from_slice(&0u32.to_le_bytes()); // locals
        data.extend_from_slice(&1u32.to_le_bytes()); // code_len
        data.push(0xFF); // invalid opcode

        let err = deserialize_module(&data).unwrap_err();
        assert!(matches!(err, SerError::UnknownOpcode(0xFF)));
    }

    // -- heap type roundtrip tests --

    #[test]
    fn function_roundtrip_with_string() {
        let mut heap = Heap::new();
        let s = heap.alloc_string("hello world".into());
        let func = Function::new(
            "str_test",
            0,
            vec![OpCode::Push(Value::String(s)), OpCode::Halt],
            0,
        );
        let bytes = serialize_function(&func, &heap);
        let mut restore_heap = Heap::new();
        let restored = deserialize_function(&bytes, &mut restore_heap).unwrap();
        assert_eq!(restored.code.len(), 2);
        if let OpCode::Push(Value::String(gc)) = &restored.code[0] {
            assert_eq!(restore_heap.get(*gc).unwrap().data, "hello world");
        } else {
            panic!("expected Push(String)");
        }
    }

    #[test]
    fn function_roundtrip_with_empty_string() {
        let mut heap = Heap::new();
        let s = heap.alloc_string(String::new());
        let func = Function::new(
            "empty",
            0,
            vec![OpCode::Push(Value::String(s)), OpCode::Halt],
            0,
        );
        let bytes = serialize_function(&func, &heap);
        let mut restore_heap = Heap::new();
        let restored = deserialize_function(&bytes, &mut restore_heap).unwrap();
        if let OpCode::Push(Value::String(gc)) = &restored.code[0] {
            assert_eq!(restore_heap.get(*gc).unwrap().data, "");
        } else {
            panic!("expected Push(String)");
        }
    }

    #[test]
    fn function_roundtrip_with_list() {
        let mut heap = Heap::new();
        let lst = heap.alloc_list(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        let func = Function::new(
            "list_test",
            0,
            vec![OpCode::Push(Value::List(lst)), OpCode::Halt],
            0,
        );
        let bytes = serialize_function(&func, &heap);
        let mut restore_heap = Heap::new();
        let restored = deserialize_function(&bytes, &mut restore_heap).unwrap();
        if let OpCode::Push(Value::List(gc)) = &restored.code[0] {
            assert_eq!(
                restore_heap.get(*gc).unwrap().elements,
                vec![Value::Int(1), Value::Int(2), Value::Int(3)]
            );
        } else {
            panic!("expected Push(List)");
        }
    }

    #[test]
    fn function_roundtrip_with_nested_list() {
        let mut heap = Heap::new();
        let inner = heap.alloc_list(vec![Value::Int(1), Value::Int(2)]);
        let outer = heap.alloc_list(vec![Value::List(inner), Value::Int(3)]);
        let func = Function::new(
            "nested",
            0,
            vec![OpCode::Push(Value::List(outer)), OpCode::Halt],
            0,
        );
        let bytes = serialize_function(&func, &heap);
        let mut restore_heap = Heap::new();
        let restored = deserialize_function(&bytes, &mut restore_heap).unwrap();
        if let OpCode::Push(Value::List(gc)) = &restored.code[0] {
            let outer_list = restore_heap.get(*gc).unwrap();
            assert_eq!(outer_list.elements.len(), 2);
            if let Value::List(inner_gc) = &outer_list.elements[0] {
                assert_eq!(
                    restore_heap.get(*inner_gc).unwrap().elements,
                    vec![Value::Int(1), Value::Int(2)]
                );
            } else {
                panic!("expected nested List");
            }
        } else {
            panic!("expected Push(List)");
        }
    }

    #[test]
    fn module_roundtrip_with_strings() {
        let mut heap = Heap::new();
        let s1 = heap.alloc_string("main".into());
        let s2 = heap.alloc_string("helper".into());
        let f1 = Function::new(
            "a",
            0,
            vec![OpCode::Push(Value::String(s1)), OpCode::Halt],
            0,
        );
        let f2 = Function::new(
            "b",
            0,
            vec![OpCode::Push(Value::String(s2)), OpCode::Halt],
            0,
        );

        let bytes = serialize_module(&[f1.clone(), f2.clone()], &heap);
        let (restored, restore_heap) = deserialize_module(&bytes).unwrap();

        assert_eq!(restored.len(), 2);
        if let OpCode::Push(Value::String(gc)) = &restored[0].code[0] {
            assert_eq!(restore_heap.get(*gc).unwrap().data, "main");
        } else {
            panic!("expected Push(String) in f1");
        }
        if let OpCode::Push(Value::String(gc)) = &restored[1].code[0] {
            assert_eq!(restore_heap.get(*gc).unwrap().data, "helper");
        } else {
            panic!("expected Push(String) in f2");
        }
    }

    #[test]
    fn function_roundtrip_with_io_opcodes() {
        let func = Function::new(
            "io_test",
            0,
            vec![
                OpCode::FileOpen {
                    path: Value::Null,
                    mode: Value::Null,
                },
                OpCode::FileRead(HandleId(0)),
                OpCode::FileWrite(HandleId(0)),
                OpCode::FileSeek {
                    handle: HandleId(0),
                    offset: Value::Int(10),
                },
                OpCode::FileClose(HandleId(0)),
                OpCode::TcpConnect { addr: Value::Null },
                OpCode::TcpRead(HandleId(1)),
                OpCode::TcpWrite(HandleId(1)),
                OpCode::TcpClose(HandleId(1)),
                OpCode::HttpGet { url: Value::Null },
                OpCode::HttpPost {
                    url: Value::Null,
                    body: Value::Null,
                },
                OpCode::StdinRead,
                OpCode::StdoutWrite,
                OpCode::StderrWrite,
                OpCode::TimerSleep {
                    ms: Value::Int(500),
                },
                OpCode::Halt,
            ],
            0,
        );
        let heap = Heap::new();
        let bytes = serialize_function(&func, &heap);
        let mut restored_heap = Heap::new();
        let restored = deserialize_function(&bytes, &mut restored_heap).unwrap();
        assert_eq!(restored.name, "io_test");
        assert_eq!(restored.arity, 0);
        assert_eq!(restored.code, func.code);
    }
}
