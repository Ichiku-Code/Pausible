use crate::opcode::OpCode;

/// A compiled function in the Pausible VM.
#[derive(Debug, Clone)]
pub struct Function {
    /// Human-readable name for debugging.
    pub name: String,
    /// Number of parameters (popped from stack on call).
    pub arity: usize,
    /// Bytecode instructions.
    pub code: Vec<OpCode>,
    /// Total local variable slots (includes parameters).
    pub num_locals: usize,
}

impl Function {
    #[must_use]
    pub fn new(name: &str, arity: usize, code: Vec<OpCode>, num_locals: usize) -> Self {
        Self {
            name: name.into(),
            arity,
            code,
            num_locals,
        }
    }
}
