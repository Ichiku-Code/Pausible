use core::fmt;

use crate::value::Value;
use crate::io::HandleId;

/// Bytecode instruction for the Pausible stack VM.
///
/// Instructions that carry an argument embed it directly. The `usize`
/// arguments are bytecode offsets (for jumps) or indices (for locals
/// and function calls).
#[derive(Debug, Clone, PartialEq)]
pub enum OpCode {
    // -- stack --
    /// Push an immediate value onto the operand stack.
    Push(Value),
    /// Discard the top of the operand stack.
    Pop,
    /// Duplicate the top of the operand stack.
    Dup,

    // -- arithmetic --
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,

    // -- comparison --
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,

    // -- logical --
    And,
    Or,
    Not,

    // -- control flow --
    /// Unconditional jump to an absolute bytecode offset.
    Jump(usize),
    /// Pop the stack; if truthy, jump to offset.
    JumpIfTrue(usize),
    /// Pop the stack; if falsy, jump to offset.
    JumpIfFalse(usize),

    // -- locals --
    /// Push the value at local slot `index` onto the operand stack.
    Load(usize),
    /// Pop the operand stack and store into local slot `index`.
    Store(usize),

    // -- functions --
    /// Call the function at the given index in the function table.
    Call(usize),
    /// Return from the current function (pop frame, push return value).
    Return,
    /// Stop execution of the VM.
    Halt,
    /// Pause execution and yield control to the host.
    Yield,

    // -- I/O: file --
    /// Open a file at  with the given  ("r"/"w"/"a"), push `HandleId`.
    FileOpen { path: Value, mode: Value },
    /// Read from file handle, push bytes as List on stack.
    FileRead(HandleId),
    /// Write data (popped from stack) to file handle.
    FileWrite(HandleId),
    /// Seek file handle to , push the resulting position.
    FileSeek { handle: HandleId, offset: Value },
    /// Close file handle, push success bool.
    FileClose(HandleId),

    // -- I/O: TCP --
    TcpConnect { addr: Value },
    TcpRead(HandleId),
    TcpWrite(HandleId),
    TcpClose(HandleId),

    // -- I/O: HTTP --
    HttpGet { url: Value },
    HttpPost { url: Value, body: Value },

    // -- I/O: standard streams --
    StdinRead,
    StdoutWrite,
    StderrWrite,

    // -- I/O: timer --
    TimerSleep { ms: Value },
}

impl OpCode {
    /// Human-readable mnemonic (without argument).
    #[must_use]
    pub fn mnemonic(&self) -> &'static str {
        match self {
            Self::Push(_) => "push",
            Self::Pop => "pop",
            Self::Dup => "dup",
            Self::Add => "add",
            Self::Sub => "sub",
            Self::Mul => "mul",
            Self::Div => "div",
            Self::Mod => "mod",
            Self::Neg => "neg",
            Self::Eq => "eq",
            Self::Neq => "neq",
            Self::Lt => "lt",
            Self::Gt => "gt",
            Self::Lte => "lte",
            Self::Gte => "gte",
            Self::And => "and",
            Self::Or => "or",
            Self::Not => "not",
            Self::Jump(_) => "jump",
            Self::JumpIfTrue(_) => "jump_if_true",
            Self::JumpIfFalse(_) => "jump_if_false",
            Self::Load(_) => "load",
            Self::Store(_) => "store",
            Self::Call(_) => "call",
            Self::Return => "ret",
            Self::Halt => "halt",
            Self::Yield => "yield",
            Self::FileOpen { .. } => "file_open",
            Self::FileRead(_) => "file_read",
            Self::FileWrite(_) => "file_write",
            Self::FileSeek { .. } => "file_seek",
            Self::FileClose(_) => "file_close",
            Self::TcpConnect { .. } => "tcp_connect",
            Self::TcpRead(_) => "tcp_read",
            Self::TcpWrite(_) => "tcp_write",
            Self::TcpClose(_) => "tcp_close",
            Self::HttpGet { .. } => "http_get",
            Self::HttpPost { .. } => "http_post",
            Self::StdinRead => "stdin_read",
            Self::StdoutWrite => "stdout_write",
            Self::StderrWrite => "stderr_write",
            Self::TimerSleep { .. } => "timer_sleep",
        }
    }
}

impl fmt::Display for OpCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Push(v) => write!(f, "push {v}"),
            Self::Jump(off) => write!(f, "jump {off}"),
            Self::JumpIfTrue(off) => write!(f, "jump_if_true {off}"),
            Self::JumpIfFalse(off) => write!(f, "jump_if_false {off}"),
            Self::Load(idx) => write!(f, "load {idx}"),
            Self::Store(idx) => write!(f, "store {idx}"),
            Self::Call(idx) => write!(f, "call {idx}"),
            Self::FileOpen { path, mode } => write!(f, "file_open {path} {mode}"),
            Self::FileRead(h) => write!(f, "file_read {h}"),
            Self::FileWrite(h) => write!(f, "file_write {h}"),
            Self::FileSeek { handle, offset } => write!(f, "file_seek {handle} {offset}"),
            Self::FileClose(h) => write!(f, "file_close {h}"),
            Self::TcpConnect { addr } => write!(f, "tcp_connect {addr}"),
            Self::TcpRead(h) => write!(f, "tcp_read {h}"),
            Self::TcpWrite(h) => write!(f, "tcp_write {h}"),
            Self::TcpClose(h) => write!(f, "tcp_close {h}"),
            Self::HttpGet { url } => write!(f, "http_get {url}"),
            Self::HttpPost { url, body } => write!(f, "http_post {url} {body}"),
            Self::StdinRead => write!(f, "stdin_read"),
            Self::StdoutWrite => write!(f, "stdout_write"),
            Self::StderrWrite => write!(f, "stderr_write"),
            Self::TimerSleep { ms } => write!(f, "timer_sleep {ms}"),
            other => write!(f, "{}", other.mnemonic()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonic_names() {
        assert_eq!(OpCode::Push(Value::Int(1)).mnemonic(), "push");
        assert_eq!(OpCode::Pop.mnemonic(), "pop");
        assert_eq!(OpCode::Dup.mnemonic(), "dup");
        assert_eq!(OpCode::Add.mnemonic(), "add");
        assert_eq!(OpCode::Sub.mnemonic(), "sub");
        assert_eq!(OpCode::Mul.mnemonic(), "mul");
        assert_eq!(OpCode::Div.mnemonic(), "div");
        assert_eq!(OpCode::Mod.mnemonic(), "mod");
        assert_eq!(OpCode::Neg.mnemonic(), "neg");
        assert_eq!(OpCode::Eq.mnemonic(), "eq");
        assert_eq!(OpCode::Neq.mnemonic(), "neq");
        assert_eq!(OpCode::Lt.mnemonic(), "lt");
        assert_eq!(OpCode::Gt.mnemonic(), "gt");
        assert_eq!(OpCode::Lte.mnemonic(), "lte");
        assert_eq!(OpCode::Gte.mnemonic(), "gte");
        assert_eq!(OpCode::And.mnemonic(), "and");
        assert_eq!(OpCode::Or.mnemonic(), "or");
        assert_eq!(OpCode::Not.mnemonic(), "not");
        assert_eq!(OpCode::Jump(0).mnemonic(), "jump");
        assert_eq!(OpCode::JumpIfTrue(1).mnemonic(), "jump_if_true");
        assert_eq!(OpCode::JumpIfFalse(2).mnemonic(), "jump_if_false");
        assert_eq!(OpCode::Load(0).mnemonic(), "load");
        assert_eq!(OpCode::Store(0).mnemonic(), "store");
        assert_eq!(OpCode::Call(0).mnemonic(), "call");
        assert_eq!(OpCode::Return.mnemonic(), "ret");
        assert_eq!(OpCode::Halt.mnemonic(), "halt");
        assert_eq!(OpCode::Yield.mnemonic(), "yield");
        assert_eq!(OpCode::FileOpen { path: Value::Null, mode: Value::Null }.mnemonic(), "file_open");
        assert_eq!(OpCode::FileRead(HandleId(0)).mnemonic(), "file_read");
        assert_eq!(OpCode::FileWrite(HandleId(0)).mnemonic(), "file_write");
        assert_eq!(OpCode::FileSeek { handle: HandleId(0), offset: Value::Null }.mnemonic(), "file_seek");
        assert_eq!(OpCode::FileClose(HandleId(0)).mnemonic(), "file_close");
        assert_eq!(OpCode::TcpConnect { addr: Value::Null }.mnemonic(), "tcp_connect");
        assert_eq!(OpCode::TcpRead(HandleId(0)).mnemonic(), "tcp_read");
        assert_eq!(OpCode::TcpWrite(HandleId(0)).mnemonic(), "tcp_write");
        assert_eq!(OpCode::TcpClose(HandleId(0)).mnemonic(), "tcp_close");
        assert_eq!(OpCode::HttpGet { url: Value::Null }.mnemonic(), "http_get");
        assert_eq!(OpCode::HttpPost { url: Value::Null, body: Value::Null }.mnemonic(), "http_post");
        assert_eq!(OpCode::StdinRead.mnemonic(), "stdin_read");
        assert_eq!(OpCode::StdoutWrite.mnemonic(), "stdout_write");
        assert_eq!(OpCode::StderrWrite.mnemonic(), "stderr_write");
        assert_eq!(OpCode::TimerSleep { ms: Value::Null }.mnemonic(), "timer_sleep");
    }

    #[test]
    fn display_with_args() {
        assert_eq!(format!("{}", OpCode::Push(Value::Int(42))), "push 42");
        assert_eq!(format!("{}", OpCode::Jump(5)), "jump 5");
        assert_eq!(format!("{}", OpCode::JumpIfTrue(10)), "jump_if_true 10");
        assert_eq!(format!("{}", OpCode::JumpIfFalse(15)), "jump_if_false 15");
        assert_eq!(format!("{}", OpCode::Load(2)), "load 2");
        assert_eq!(format!("{}", OpCode::Store(3)), "store 3");
        assert_eq!(format!("{}", OpCode::Call(7)), "call 7");
        assert_eq!(format!("{}", OpCode::FileOpen { path: Value::Int(1), mode: Value::Int(2) }), "file_open 1 2");
        assert_eq!(format!("{}", OpCode::FileRead(HandleId(3))), "file_read #3");
        assert_eq!(format!("{}", OpCode::FileWrite(HandleId(4))), "file_write #4");
        assert_eq!(format!("{}", OpCode::FileSeek { handle: HandleId(5), offset: Value::Int(100) }), "file_seek #5 100");
        assert_eq!(format!("{}", OpCode::FileClose(HandleId(6))), "file_close #6");
        assert_eq!(format!("{}", OpCode::TcpConnect { addr: Value::Null }), "tcp_connect null");
        assert_eq!(format!("{}", OpCode::HttpGet { url: Value::Null }), "http_get null");
        assert_eq!(format!("{}", OpCode::HttpPost { url: Value::Null, body: Value::Null }), "http_post null null");
        assert_eq!(format!("{}", OpCode::TimerSleep { ms: Value::Int(500) }), "timer_sleep 500");
        assert_eq!(format!("{}", OpCode::StdinRead), "stdin_read");
    }

    #[test]
    fn display_no_args() {
        assert_eq!(format!("{}", OpCode::Pop), "pop");
        assert_eq!(format!("{}", OpCode::Add), "add");
        assert_eq!(format!("{}", OpCode::Return), "ret");
        assert_eq!(format!("{}", OpCode::Halt), "halt");
        assert_eq!(format!("{}", OpCode::Yield), "yield");
        assert_eq!(format!("{}", OpCode::StdoutWrite), "stdout_write");
        assert_eq!(format!("{}", OpCode::StderrWrite), "stderr_write");
    }

    #[test]
    fn total_instruction_count() {
        let variants: &[OpCode] = &[
            // -- stack --
            OpCode::Push(Value::Null), OpCode::Pop, OpCode::Dup,
            // -- arithmetic --
            OpCode::Add, OpCode::Sub, OpCode::Mul, OpCode::Div, OpCode::Mod, OpCode::Neg,
            // -- comparison --
            OpCode::Eq, OpCode::Neq, OpCode::Lt, OpCode::Gt, OpCode::Lte, OpCode::Gte,
            // -- logical --
            OpCode::And, OpCode::Or, OpCode::Not,
            // -- control flow --
            OpCode::Jump(0), OpCode::JumpIfTrue(0), OpCode::JumpIfFalse(0),
            // -- locals --
            OpCode::Load(0), OpCode::Store(0),
            // -- functions --
            OpCode::Call(0), OpCode::Return,
            OpCode::Halt, OpCode::Yield,
            // -- I/O: file (5) --
            OpCode::FileOpen { path: Value::Null, mode: Value::Null },
            OpCode::FileRead(HandleId(0)),
            OpCode::FileWrite(HandleId(0)),
            OpCode::FileSeek { handle: HandleId(0), offset: Value::Null },
            OpCode::FileClose(HandleId(0)),
            // -- I/O: TCP (4) --
            OpCode::TcpConnect { addr: Value::Null },
            OpCode::TcpRead(HandleId(0)),
            OpCode::TcpWrite(HandleId(0)),
            OpCode::TcpClose(HandleId(0)),
            // -- I/O: HTTP (2) --
            OpCode::HttpGet { url: Value::Null },
            OpCode::HttpPost { url: Value::Null, body: Value::Null },
            // -- I/O: std streams (3) --
            OpCode::StdinRead, OpCode::StdoutWrite, OpCode::StderrWrite,
            // -- I/O: timer (1) --
            OpCode::TimerSleep { ms: Value::Null },
        ];
        // 27 existing + 15 new = 42
        assert_eq!(variants.len(), 42);
    }

    #[test]
    fn equality() {
        assert_eq!(OpCode::Push(Value::Int(1)), OpCode::Push(Value::Int(1)));
        assert_ne!(OpCode::Push(Value::Int(1)), OpCode::Push(Value::Int(2)));
        assert_eq!(OpCode::Add, OpCode::Add);
        assert_ne!(OpCode::Add, OpCode::Sub);
        // I/O variants with HandleId
        assert_eq!(OpCode::FileRead(HandleId(1)), OpCode::FileRead(HandleId(1)));
        assert_ne!(OpCode::FileRead(HandleId(1)), OpCode::FileRead(HandleId(2)));
    }
}
