use pausible::function::Function;
use pausible::opcode::OpCode;
use pausible::value::Value;
use pausible::vm::VM;

fn run_program(functions: Vec<Function>) -> VM {
    let mut vm = VM::new();
    for func in functions {
        vm.add_function(func);
    }
    vm.prepare(0).unwrap();
    vm.run().unwrap();
    vm
}

// -- 斐波那契（递归） --

#[test]
fn fibonacci_recursive() {
    // fib(0) = 0, fib(1) = 1, fib(n) = fib(n-1) + fib(n-2)
    let fib = Function::new(
        "fib",
        1,
        vec![
            OpCode::Load(0),             // 0: push n
            OpCode::Push(Value::Int(1)), // 1: push 1
            OpCode::Lte,                 // 2: n <= 1?
            OpCode::JumpIfTrue(14),      // 3: if true → base case
            OpCode::Load(0),             // 4: push n
            OpCode::Push(Value::Int(1)), // 5: push 1
            OpCode::Sub,                 // 6: n-1
            OpCode::Call(1),             // 7: fib(n-1)
            OpCode::Load(0),             // 8: push n
            OpCode::Push(Value::Int(2)), // 9: push 2
            OpCode::Sub,                 // 10: n-2
            OpCode::Call(1),             // 11: fib(n-2)
            OpCode::Add,                 // 12: fib(n-1)+fib(n-2)
            OpCode::Return,              // 13: return
            // base case:
            OpCode::Load(0), // 14: push n
            OpCode::Return,  // 15: return n
        ],
        1,
    );

    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(10)),
            OpCode::Call(1), // fib(10)
            OpCode::Halt,
        ],
        0,
    );

    let vm = run_program(vec![main, fib]);
    // fib(10) = 55
    assert_eq!(vm.stack, vec![Value::Int(55)]);
}

// -- 阶乘（递归） --

#[test]
fn factorial_recursive() {
    // fact(0) = 1, fact(1) = 1, fact(n) = n * fact(n-1)
    let fact = Function::new(
        "fact",
        1,
        vec![
            OpCode::Load(0),             // 0: push n
            OpCode::Push(Value::Int(1)), // 1: push 1
            OpCode::Lte,                 // 2: n <= 1?
            OpCode::JumpIfTrue(11),      // 3: if true → base case
            OpCode::Load(0),             // 4: push n
            OpCode::Load(0),             // 5: push n
            OpCode::Push(Value::Int(1)), // 6: push 1
            OpCode::Sub,                 // 7: n-1
            OpCode::Call(1),             // 8: fact(n-1)
            OpCode::Mul,                 // 9: n * fact(n-1)
            OpCode::Return,              // 10: return
            // base case:
            OpCode::Push(Value::Int(1)), // 11: push 1
            OpCode::Return,              // 12: return 1
        ],
        1,
    );

    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(5)),
            OpCode::Call(1), // fact(5)
            OpCode::Halt,
        ],
        0,
    );

    let vm = run_program(vec![main, fact]);
    // fact(5) = 120
    assert_eq!(vm.stack, vec![Value::Int(120)]);
}

// -- 带循环的求和 --

#[test]
fn loop_sum() {
    // sum(5) = 1+2+3+4+5 = 15
    // locals: 0=n, 1=result, 2=i
    let sum = Function::new(
        "sum",
        1,
        vec![
            OpCode::Push(Value::Int(0)), // 0: push 0
            OpCode::Store(1),            // 1: result = 0
            OpCode::Load(0),             // 2: push n
            OpCode::Store(2),            // 3: i = n
            // loop:
            OpCode::Load(2),             // 4: push i
            OpCode::Push(Value::Int(0)), // 5: push 0
            OpCode::Gt,                  // 6: i > 0?
            OpCode::JumpIfFalse(17),     // 7: if false → done
            OpCode::Load(1),             // 8: push result
            OpCode::Load(2),             // 9: push i
            OpCode::Add,                 // 10: result + i
            OpCode::Store(1),            // 11: result = result + i
            OpCode::Load(2),             // 12: push i
            OpCode::Push(Value::Int(1)), // 13: push 1
            OpCode::Sub,                 // 14: i - 1
            OpCode::Store(2),            // 15: i = i - 1
            OpCode::Jump(4),             // 16: loop back
            // done:
            OpCode::Load(1), // 17: push result
            OpCode::Return,  // 18: return
        ],
        3,
    );

    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(5)),
            OpCode::Call(1), // sum(5)
            OpCode::Halt,
        ],
        0,
    );

    let vm = run_program(vec![main, sum]);
    assert_eq!(vm.stack, vec![Value::Int(15)]);
}

// -- 条件分支（if/else） --

#[test]
fn conditional_abs() {
    // abs(n): if n >= 0 return n else return -n
    let abs = Function::new(
        "abs",
        1,
        vec![
            OpCode::Load(0),             // 0: push n
            OpCode::Push(Value::Int(0)), // 1: push 0
            OpCode::Gte,                 // 2: n >= 0?
            OpCode::JumpIfTrue(7),       // 3: if true → if branch
            // else:
            OpCode::Load(0), // 4: push n
            OpCode::Neg,     // 5: -n
            OpCode::Jump(8), // 6: skip if branch
            // if:
            OpCode::Load(0), // 7: push n
            // return:
            OpCode::Return, // 8: return
        ],
        1,
    );

    let main_neg = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(-7)),
            OpCode::Call(1), // abs(-7)
            OpCode::Halt,
        ],
        0,
    );

    let main_pos = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(3)),
            OpCode::Call(1), // abs(3)
            OpCode::Halt,
        ],
        0,
    );

    let vm_neg = run_program(vec![main_neg, abs.clone()]);
    assert_eq!(vm_neg.stack, vec![Value::Int(7)]);

    let vm_pos = run_program(vec![main_pos, abs]);
    assert_eq!(vm_pos.stack, vec![Value::Int(3)]);
}

// -- 嵌套函数调用 --

#[test]
fn nested_compose() {
    // square(double(3)) = 36
    let square = Function::new(
        "square",
        1,
        vec![
            OpCode::Load(0), // x
            OpCode::Load(0), // x
            OpCode::Mul,     // x * x
            OpCode::Return,
        ],
        1,
    );

    let double = Function::new(
        "double",
        1,
        vec![
            OpCode::Load(0),
            OpCode::Push(Value::Int(2)),
            OpCode::Mul, // x * 2
            OpCode::Return,
        ],
        1,
    );

    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(3)),
            OpCode::Call(2), // double(3) → 6
            OpCode::Call(1), // square(6) → 36
            OpCode::Halt,
        ],
        0,
    );

    let vm = run_program(vec![main, square, double]);
    assert_eq!(vm.stack, vec![Value::Int(36)]);
}

#[test]
fn yield_snapshot_resume_roundtrip() {
    use pausible::function::Function;
    use pausible::opcode::OpCode;
    use pausible::value::Value;
    use pausible::vm::VM;

    let code = vec![
        OpCode::Push(Value::Int(1)),
        OpCode::Yield,
        OpCode::Push(Value::Int(2)),
        OpCode::Yield,
        OpCode::Push(Value::Int(3)),
        OpCode::Halt,
    ];

    // Step through one instruction at a time
    let mut vm = VM::new();
    let main = Function::new("main", 0, code.clone(), 0);
    vm.add_function(main);
    vm.prepare(0).unwrap();

    vm.step().unwrap(); // push 1
    assert_eq!(vm.stack, &[Value::Int(1)]);
    assert!(vm.running);

    vm.step().unwrap(); // yield -> pauses
    assert!(!vm.running);
    assert_eq!(vm.stack, &[Value::Int(1)]);

    // Take snapshot at yield point
    let snap = vm.create_snapshot();

    // Restore into fresh VM
    let mut restored = VM::new();
    let main2 = Function::new("main", 0, code.clone(), 0);
    restored.add_function(main2);
    restored.restore_snapshot(&snap).unwrap();
    assert!(restored.running);
    assert_eq!(restored.stack, &[Value::Int(1)]);

    // Resume from yield: push 2, yield, push 3, halt
    restored.step().unwrap(); // push 2
    assert_eq!(restored.stack, &[Value::Int(1), Value::Int(2)]);
    restored.step().unwrap(); // yield -> pauses
    assert!(!restored.running);

    restored.step().unwrap(); // push 3
    restored.step().unwrap(); // halt
    assert!(!restored.running);
    assert_eq!(
        restored.stack,
        &[Value::Int(1), Value::Int(2), Value::Int(3)]
    );

    // Also test run() stops at first yield
    let mut vm3 = VM::new();
    let main3 = Function::new("main", 0, code, 0);
    vm3.add_function(main3);
    vm3.prepare(0).unwrap();
    vm3.run().unwrap();
    // run() stops at the first Yield, so only push 1 executed
    assert_eq!(vm3.stack, &[Value::Int(1)]);
}

#[test]
fn resume_from_file_roundtrip() {
    use pausible::function::Function;
    use pausible::opcode::OpCode;
    use pausible::snapshot::Snapshot;
    use pausible::value::Value;
    use pausible::vm::VM;

    let code = vec![
        OpCode::Push(Value::Int(10)),
        OpCode::Push(Value::Int(20)),
        OpCode::Yield,
        OpCode::Push(Value::Int(30)),
        OpCode::Halt,
    ];

    // Execute to yield point
    let mut vm = VM::new();
    let main = Function::new("main", 0, code.clone(), 0);
    vm.add_function(main);
    vm.prepare(0).unwrap();

    vm.step().unwrap(); // push 10
    vm.step().unwrap(); // push 20
    vm.step().unwrap(); // yield → pause
    assert!(!vm.running);
    assert_eq!(vm.stack, &[Value::Int(10), Value::Int(20)]);

    // Save snapshot to file
    let snap = vm.create_snapshot();
    let path = "/tmp/pausible_demo_snapshot.bin";
    snap.write_to_file(path).unwrap();

    // Load snapshot from file and resume
    let loaded = Snapshot::read_from_file(path).unwrap();
    let err = vm.resume(&loaded);
    assert!(err.is_ok(), "resume from file should succeed");

    // After resume, run continued until Halt
    assert!(!vm.running);
    assert_eq!(vm.stack, &[Value::Int(10), Value::Int(20), Value::Int(30)]);
}

#[test]
fn resume_with_string_heap_objects() {
    use pausible::function::Function;
    use pausible::opcode::OpCode;
    use pausible::snapshot::Snapshot;
    use pausible::value::Value;
    use pausible::vm::VM;

    // Simple: push a heap string, yield, then push another and halt
    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(0)), // placeholder, replaced below
            OpCode::Yield,
            OpCode::Push(Value::Int(0)), // placeholder
            OpCode::Halt,
        ],
        0,
    );

    let mut vm = VM::new();
    vm.add_function(main);

    let hello_gc = vm.alloc_string("Hello".into());
    let world_gc = vm.alloc_string("World".into());

    vm.prepare(0).unwrap();

    vm.step().unwrap(); // Push(Int(0)) → IP=1
    vm.step().unwrap(); // Yield → pause, IP=2

    assert!(!vm.running);

    // Replace stack values with heap strings
    vm.stack[0] = Value::String(hello_gc);
    vm.stack.push(Value::String(world_gc));

    // Snapshot at yield point
    let snap = vm.create_snapshot();
    let path = "/tmp/pausible_string_heap.bin";
    snap.write_to_file(path).unwrap();

    // Resume from file
    let loaded = Snapshot::read_from_file(path).unwrap();
    let err = vm.resume(&loaded);
    assert!(err.is_ok(), "resume should succeed");

    assert!(!vm.running);
    // Stack should still have our heap strings + new values from post-yield
    assert!(vm.stack.len() >= 2, "stack should have heap strings");
    if let Value::String(gc) = &vm.stack[0] {
        assert_eq!(vm.heap.get(*gc).unwrap().data, "Hello");
    } else {
        panic!("expected String");
    }
}

// -- Phase 2.7: 跨架构验证与测试 --

// 斐波那契 yield 中途保存，恢复后得出正确结果
#[test]
fn fibonacci_yield_midpoint_resume() {
    use pausible::function::Function;
    use pausible::opcode::OpCode;
    use pausible::value::Value;
    use pausible::vm::VM;

    // Iterative fib(6) = 8, yield when counter hits 3 (midpoint)
    // locals: 0=counter, 1=a, 2=b, 3=temp(c)
    let fib = Function::new(
        "fib",
        0,
        vec![
            // Setup: counter=5, a=0, b=1
            OpCode::Push(Value::Int(5)), // 0: counter = 5
            OpCode::Store(0),            // 1
            OpCode::Push(Value::Int(0)), // 2: a = 0
            OpCode::Store(1),            // 3
            OpCode::Push(Value::Int(1)), // 4: b = 1
            OpCode::Store(2),            // 5
            // loop:
            OpCode::Load(0),             // 6: push counter
            OpCode::Push(Value::Int(0)), // 7
            OpCode::Eq,                  // 8: counter == 0?
            OpCode::JumpIfTrue(28),      // 9: if so, goto end
            OpCode::Load(0),             // 10: push counter
            OpCode::Push(Value::Int(3)), // 11
            OpCode::Eq,                  // 12: counter == 3?
            OpCode::JumpIfFalse(15),     // 13: skip yield
            OpCode::Yield,               // 14: yield at midpoint
            OpCode::Load(1),             // 15: push a
            OpCode::Load(2),             // 16: push b
            OpCode::Add,                 // 17: c = a+b
            OpCode::Store(3),            // 18: local[3] = c
            OpCode::Load(2),             // 19: push b
            OpCode::Store(1),            // 20: a = b
            OpCode::Load(3),             // 21: push c
            OpCode::Store(2),            // 22: b = c
            OpCode::Load(0),             // 23: push counter
            OpCode::Push(Value::Int(1)), // 24
            OpCode::Sub,                 // 25: counter - 1
            OpCode::Store(0),            // 26: counter = counter-1
            OpCode::Jump(6),             // 27: loop
            // end:
            OpCode::Load(2), // 28: push result (b)
            OpCode::Halt,    // 29
        ],
        4, // 4 locals
    );

    let mut vm = VM::new();
    vm.add_function(fib);
    vm.prepare(0).unwrap();

    vm.run().unwrap();
    assert!(!vm.running, "should have yielded");

    // At this point: counter=3, a=1, b=2 (fib(2)=1, fib(3)=2)
    let snap = vm.create_snapshot();

    // Resume: compute remaining terms (fib(4)=3, fib(5)=5, fib(6)=8)
    vm.resume(&snap).unwrap();
    assert!(!vm.running, "should have halted");

    // Result: fib(6) = 8
    assert_eq!(vm.stack, &[Value::Int(8)]);
}

// 阶乘嵌套 yield（多次保存/恢复）
#[test]
fn factorial_nested_yields() {
    use pausible::function::Function;
    use pausible::opcode::OpCode;
    use pausible::value::Value;
    use pausible::vm::VM;

    // Iterative factorial: fact(5) = 120, yield every iteration
    // locals: 0=counter, 1=result
    let fact = Function::new(
        "fact",
        0,
        vec![
            // Setup: counter=5, result=1
            OpCode::Push(Value::Int(5)), // 0
            OpCode::Store(0),            // 1: counter = 5
            OpCode::Push(Value::Int(1)), // 2
            OpCode::Store(1),            // 3: result = 1
            // loop:
            OpCode::Load(0),             // 4: push counter
            OpCode::Push(Value::Int(0)), // 5
            OpCode::Eq,                  // 6: counter == 0?
            OpCode::JumpIfTrue(22),      // 7: yes -> end
            OpCode::Load(1),             // 8: push result
            OpCode::Load(0),             // 9: push counter
            OpCode::Mul,                 // 10: result * counter
            OpCode::Store(1),            // 11: result = result * counter
            OpCode::Load(0),             // 12: push counter
            OpCode::Push(Value::Int(1)), // 13
            OpCode::Sub,                 // 14: counter - 1
            OpCode::Store(0),            // 15: counter = counter - 1
            OpCode::Load(0),             // 16: push counter
            OpCode::Push(Value::Int(0)), // 17: push 0
            OpCode::Eq,                  // 18: counter == 0?
            OpCode::JumpIfTrue(4),       // 19: if counter==0, skip yield, loop back
            OpCode::Yield,               // 20: yield only when counter > 0
            OpCode::Jump(4),             // 21: loop back
            // end:
            OpCode::Load(1), // 22: push result
            OpCode::Halt,    // 23
        ],
        2,
    );

    let mut vm = VM::new();
    vm.add_function(fact);
    vm.prepare(0).unwrap();

    // Yield cycle 1: counter goes from 5 to 4, result=5
    vm.run().unwrap();
    assert!(!vm.running);
    assert_eq!(vm.frames[0].locals[0], Value::Int(4));
    assert_eq!(vm.frames[0].locals[1], Value::Int(5));
    let snap1 = vm.create_snapshot();

    // Yield cycle 2: counter 4->3, result=20
    vm.resume(&snap1).unwrap();
    assert!(!vm.running);
    assert_eq!(vm.frames[0].locals[0], Value::Int(3));
    assert_eq!(vm.frames[0].locals[1], Value::Int(20));
    let snap2 = vm.create_snapshot();

    // Yield cycle 3: counter 3->2, result=60
    vm.resume(&snap2).unwrap();
    assert!(!vm.running);
    assert_eq!(vm.frames[0].locals[0], Value::Int(2));
    assert_eq!(vm.frames[0].locals[1], Value::Int(60));
    let snap3 = vm.create_snapshot();

    // Yield cycle 4: counter 2->1, result=120
    vm.resume(&snap3).unwrap();
    assert!(!vm.running);
    assert_eq!(vm.frames[0].locals[0], Value::Int(1));
    assert_eq!(vm.frames[0].locals[1], Value::Int(120));
    let snap4 = vm.create_snapshot();

    // Final cycle: counter 1->0, result=120, loop exits, halt
    vm.resume(&snap4).unwrap();
    assert!(!vm.running);
    assert_eq!(vm.stack, &[Value::Int(120)]);
}

// 带循环的程序：迭代中 yield，resume 后继续
#[test]
fn loop_yield_per_iteration() {
    use pausible::function::Function;
    use pausible::opcode::OpCode;
    use pausible::value::Value;
    use pausible::vm::VM;

    // Loop that accumulates sum(1..5), yielding each iteration
    // locals: 0=i (counting down), 1=sum
    let loop_sum = Function::new(
        "sum",
        0,
        vec![
            OpCode::Push(Value::Int(5)), // 0
            OpCode::Store(0),            // 1: i = 5
            OpCode::Push(Value::Int(0)), // 2
            OpCode::Store(1),            // 3: sum = 0
            // loop:
            OpCode::Load(0),             // 4: push i
            OpCode::Push(Value::Int(0)), // 5
            OpCode::Eq,                  // 6: i == 0?
            OpCode::JumpIfTrue(22),      // 7: yes -> end
            OpCode::Load(1),             // 8: push sum
            OpCode::Load(0),             // 9: push i
            OpCode::Add,                 // 10: sum + i
            OpCode::Store(1),            // 11: sum = sum + i
            OpCode::Load(0),             // 12: push i
            OpCode::Push(Value::Int(1)), // 13
            OpCode::Sub,                 // 14: i - 1
            OpCode::Store(0),            // 15: i = i - 1
            OpCode::Load(0),             // 16: push counter
            OpCode::Push(Value::Int(0)), // 17: push 0
            OpCode::Eq,                  // 18: i == 0?
            OpCode::JumpIfTrue(4),       // 19: if i==0, skip yield, loop back
            OpCode::Yield,               // 20: yield only when i > 0
            OpCode::Jump(4),             // 21: loop back
            // end:
            OpCode::Load(1), // 22: push sum
            OpCode::Halt,    // 23
        ],
        2,
    );

    let mut vm = VM::new();
    vm.add_function(loop_sum);
    vm.prepare(0).unwrap();

    // Iteration 1: i=5, sum grows
    vm.run().unwrap();
    assert!(!vm.running);
    assert_eq!(vm.frames[0].locals[0], Value::Int(4));
    assert_eq!(vm.frames[0].locals[1], Value::Int(5));
    let snap1 = vm.create_snapshot();
    vm.resume(&snap1).unwrap();

    // Iteration 2: i=4, sum=9
    assert!(!vm.running);
    assert_eq!(vm.frames[0].locals[1], Value::Int(9));
    let snap2 = vm.create_snapshot();
    vm.resume(&snap2).unwrap();

    // Iteration 3: i=3, sum=12
    assert!(!vm.running);
    assert_eq!(vm.frames[0].locals[1], Value::Int(12));
    let snap3 = vm.create_snapshot();
    vm.resume(&snap3).unwrap();

    // Iteration 4: i=2, sum=14
    assert!(!vm.running);
    assert_eq!(vm.frames[0].locals[1], Value::Int(14));
    let snap4 = vm.create_snapshot();
    vm.resume(&snap4).unwrap();

    // Iteration 5: i=1, sum=15, then loop exits
    assert!(!vm.running);
    assert_eq!(vm.stack, &[Value::Int(15)]);
}

// 端序一致性验证（to_le_bytes / from_le_bytes）
#[test]
fn endianness_consistency() {
    use pausible::function::Function;
    use pausible::opcode::OpCode;
    use pausible::value::Value;
    use pausible::vm::VM;

    // Verify that multi-byte scalars serialize as LE and deserialize exactly.
    // i64::MIN, 0, i64::MAX each test different byte patterns.
    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(i64::MIN)),
            OpCode::Yield,
            OpCode::Push(Value::Int(0)),
            OpCode::Yield,
            OpCode::Push(Value::Int(i64::MAX)),
            OpCode::Halt,
        ],
        0,
    );

    let mut vm = VM::new();
    vm.add_function(main);
    vm.prepare(0).unwrap();
    vm.run().unwrap(); // stop at Yield after push(i64::MIN)
    assert_eq!(vm.stack, &[Value::Int(i64::MIN)]);

    let snap = vm.create_snapshot();
    vm.resume(&snap).unwrap(); // stop at Yield after push(0)
    assert_eq!(vm.stack, &[Value::Int(i64::MIN), Value::Int(0)]);

    let snap2 = vm.create_snapshot();
    vm.resume(&snap2).unwrap(); // run until Halt
    assert_eq!(
        vm.stack,
        &[Value::Int(i64::MIN), Value::Int(0), Value::Int(i64::MAX)]
    );
}

// 浮点数跨平台确定性验证（IEEE 754 原始字节存储）
#[test]
fn floating_point_cross_platform_determinism() {
    use pausible::function::Function;
    use pausible::opcode::OpCode;
    use pausible::value::Value;
    use pausible::vm::VM;

    // Verify IEEE 754 special values survive snapshot roundtrip.
    // NaN, +inf, -inf, and -0.0 are stored as raw bytes (to_le_bytes)
    // and must deserialize bit-identically.
    let specials: [f64; 6] = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        0.0_f64,
        -0.0_f64,
        f64::MIN_POSITIVE,
    ];

    // Push each value, yield, snapshot, resume, check bits.
    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Float(specials[0])),
            OpCode::Yield,
            OpCode::Push(Value::Float(specials[1])),
            OpCode::Yield,
            OpCode::Push(Value::Float(specials[2])),
            OpCode::Yield,
            OpCode::Push(Value::Float(specials[3])),
            OpCode::Yield,
            OpCode::Push(Value::Float(specials[4])),
            OpCode::Yield,
            OpCode::Push(Value::Float(specials[5])),
            OpCode::Halt,
        ],
        0,
    );

    let mut vm = VM::new();
    vm.add_function(main);
    vm.prepare(0).unwrap();

    for (i, &expected) in specials.iter().enumerate() {
        vm.run().unwrap(); // stop at Yield (or Halt on last iteration)
        // Check the bit pattern of the value we just pushed
        if let Some(Value::Float(f)) = vm.stack.get(i) {
            let bits_original = expected.to_bits();
            let bits_restored = (*f).to_bits();
            assert_eq!(
                bits_restored, bits_original,
                "float bits mismatch at index {i}: expected {bits_original:#018x}, got {bits_restored:#018x}"
            );
        }
        // Snapshot and resume for the next value (except on the last, which halts)
        if i < specials.len() - 1 {
            let snap = vm.create_snapshot();
            vm.resume(&snap).unwrap();
        }
    }

    assert_eq!(vm.stack.len(), 6);
}
