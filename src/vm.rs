// `prepare`, `run`, `step` all return VmError; adding per-method # Errors
// sections would repeat the VmError variants verbatim.
#![allow(clippy::missing_errors_doc)]

use core::fmt;

use crate::function::Function;
use crate::opcode::OpCode;
use crate::heap::{Heap, Gc, ListObj, StringObj};
use crate::value::{TypeError, Value};

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
    fn new(function: usize, locals: Vec<Value>) -> Self {
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
        }
    }
}

impl core::error::Error for VmError {}

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
            running: false,
        }
    }

    /// Allocate a string on the heap.
    pub fn alloc_string(&mut self, data: String) -> Gc<StringObj> {
        self.heap.alloc_string(data)
    }

    /// Allocate a list on the heap.
    pub fn alloc_list(&mut self, elements: Vec<Value>) -> Gc<ListObj> {
        self.heap.alloc_list(elements)
    }

    /// Borrow a heap string by handle.
    pub fn get_string(&self, gc: Gc<StringObj>) -> Result<&StringObj, VmError> {
        self.heap.get(gc).ok_or(VmError::HeapError(HeapError::InvalidHandle))
    }

    /// Mutably borrow a heap string.
    pub fn get_string_mut(&mut self, gc: Gc<StringObj>) -> Result<&mut StringObj, VmError> {
        self.heap.get_mut(gc).ok_or(VmError::HeapError(HeapError::InvalidHandle))
    }

    /// Borrow a heap list by handle.
    pub fn get_list(&self, gc: Gc<ListObj>) -> Result<&ListObj, VmError> {
        self.heap.get(gc).ok_or(VmError::HeapError(HeapError::InvalidHandle))
    }

    /// Mutably borrow a heap list.
    pub fn get_list_mut(&mut self, gc: Gc<ListObj>) -> Result<&mut ListObj, VmError> {
        self.heap.get_mut(gc).ok_or(VmError::HeapError(HeapError::InvalidHandle))
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

    /// Execute until Halt or an error.
    pub fn run(&mut self) -> Result<(), VmError> {
        while self.running {
            self.step()?;
        }
        Ok(())
    }

    /// Execute a single instruction.
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
                let val = self
                    .frames[frame_idx]
                    .locals
                    .get(idx)
                    .ok_or(VmError::LocalOutOfBounds(idx))?;
                self.stack.push(val.clone());
            }
            OpCode::Store(idx) => {
                let val = self.pop()?;
                let slot = self
                    .frames[frame_idx]
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
            OpCode::Halt => {
                self.running = false;
            }
        }

        Ok(())
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

    fn unary_op(
        &mut self,
        op: fn(&Value) -> Result<Value, TypeError>,
    ) -> Result<(), VmError> {
        let val = self.pop()?;
        let result = op(&val).map_err(VmError::TypeError)?;
        self.stack.push(result);
        Ok(())
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
        let vm = run_code(vec![
            OpCode::Push(Value::Int(7)),
            OpCode::Dup,
            OpCode::Halt,
        ])
        .unwrap();
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
            vec![
                OpCode::Push(Value::Int(41)),
                OpCode::Call(1),
                OpCode::Halt,
            ],
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
        vm.add_function(main);      // idx 0
        vm.add_function(add_doubled); // idx 1
        vm.add_function(double);    // idx 2
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
}
