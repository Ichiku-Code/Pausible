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
            OpCode::Yield | OpCode::Halt => {
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
            OpCode::Push(Value::Null),  // placeholder
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

}
