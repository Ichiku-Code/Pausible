use core::fmt;

use crate::value::Value;

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
    }

    #[test]
    fn display_no_args() {
        assert_eq!(format!("{}", OpCode::Pop), "pop");
        assert_eq!(format!("{}", OpCode::Add), "add");
        assert_eq!(format!("{}", OpCode::Return), "ret");
        assert_eq!(format!("{}", OpCode::Halt), "halt");
        assert_eq!(format!("{}", OpCode::Yield), "yield");
    }

    #[test]
    fn total_instruction_count() {
        // Count distinct variants (25)
        let variants: &[OpCode] = &[
            OpCode::Push(Value::Null),
            OpCode::Pop,
            OpCode::Dup,
            OpCode::Add,
            OpCode::Sub,
            OpCode::Mul,
            OpCode::Div,
            OpCode::Mod,
            OpCode::Neg,
            OpCode::Eq,
            OpCode::Neq,
            OpCode::Lt,
            OpCode::Gt,
            OpCode::Lte,
            OpCode::Gte,
            OpCode::And,
            OpCode::Or,
            OpCode::Not,
            OpCode::Jump(0),
            OpCode::JumpIfTrue(0),
            OpCode::JumpIfFalse(0),
            OpCode::Load(0),
            OpCode::Store(0),
            OpCode::Call(0),
            OpCode::Return,
            OpCode::Halt,
            OpCode::Yield,
        ];
        assert_eq!(variants.len(), 27);
    }

    #[test]
    fn equality() {
        assert_eq!(OpCode::Push(Value::Int(1)), OpCode::Push(Value::Int(1)));
        assert_ne!(OpCode::Push(Value::Int(1)), OpCode::Push(Value::Int(2)));
        assert_eq!(OpCode::Add, OpCode::Add);
        assert_ne!(OpCode::Add, OpCode::Sub);
    }
}
