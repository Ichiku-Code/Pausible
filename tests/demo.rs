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
            OpCode::Load(0),              // 0: push n
            OpCode::Push(Value::Int(1)),  // 1: push 1
            OpCode::Lte,                  // 2: n <= 1?
            OpCode::JumpIfTrue(14),       // 3: if true → base case
            OpCode::Load(0),              // 4: push n
            OpCode::Push(Value::Int(1)),  // 5: push 1
            OpCode::Sub,                  // 6: n-1
            OpCode::Call(1),              // 7: fib(n-1)
            OpCode::Load(0),              // 8: push n
            OpCode::Push(Value::Int(2)),  // 9: push 2
            OpCode::Sub,                  // 10: n-2
            OpCode::Call(1),              // 11: fib(n-2)
            OpCode::Add,                  // 12: fib(n-1)+fib(n-2)
            OpCode::Return,               // 13: return
            // base case:
            OpCode::Load(0),              // 14: push n
            OpCode::Return,               // 15: return n
        ],
        1,
    );

    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(10)),
            OpCode::Call(1),  // fib(10)
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
            OpCode::Load(0),              // 0: push n
            OpCode::Push(Value::Int(1)),  // 1: push 1
            OpCode::Lte,                  // 2: n <= 1?
            OpCode::JumpIfTrue(11),       // 3: if true → base case
            OpCode::Load(0),              // 4: push n
            OpCode::Load(0),              // 5: push n
            OpCode::Push(Value::Int(1)),  // 6: push 1
            OpCode::Sub,                  // 7: n-1
            OpCode::Call(1),              // 8: fact(n-1)
            OpCode::Mul,                  // 9: n * fact(n-1)
            OpCode::Return,               // 10: return
            // base case:
            OpCode::Push(Value::Int(1)),  // 11: push 1
            OpCode::Return,               // 12: return 1
        ],
        1,
    );

    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(5)),
            OpCode::Call(1),  // fact(5)
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
            OpCode::Push(Value::Int(0)),  // 0: push 0
            OpCode::Store(1),             // 1: result = 0
            OpCode::Load(0),              // 2: push n
            OpCode::Store(2),             // 3: i = n
            // loop:
            OpCode::Load(2),              // 4: push i
            OpCode::Push(Value::Int(0)),  // 5: push 0
            OpCode::Gt,                   // 6: i > 0?
            OpCode::JumpIfFalse(17),      // 7: if false → done
            OpCode::Load(1),              // 8: push result
            OpCode::Load(2),              // 9: push i
            OpCode::Add,                  // 10: result + i
            OpCode::Store(1),             // 11: result = result + i
            OpCode::Load(2),              // 12: push i
            OpCode::Push(Value::Int(1)),  // 13: push 1
            OpCode::Sub,                  // 14: i - 1
            OpCode::Store(2),             // 15: i = i - 1
            OpCode::Jump(4),              // 16: loop back
            // done:
            OpCode::Load(1),              // 17: push result
            OpCode::Return,               // 18: return
        ],
        3,
    );

    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(5)),
            OpCode::Call(1),  // sum(5)
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
            OpCode::Load(0),              // 0: push n
            OpCode::Push(Value::Int(0)),  // 1: push 0
            OpCode::Gte,                  // 2: n >= 0?
            OpCode::JumpIfTrue(7),        // 3: if true → if branch
            // else:
            OpCode::Load(0),              // 4: push n
            OpCode::Neg,                  // 5: -n
            OpCode::Jump(8),              // 6: skip if branch
            // if:
            OpCode::Load(0),              // 7: push n
            // return:
            OpCode::Return,               // 8: return
        ],
        1,
    );

    let main_neg = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(-7)),
            OpCode::Call(1),  // abs(-7)
            OpCode::Halt,
        ],
        0,
    );

    let main_pos = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(3)),
            OpCode::Call(1),  // abs(3)
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
            OpCode::Load(0),   // x
            OpCode::Load(0),   // x
            OpCode::Mul,       // x * x
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
            OpCode::Mul,       // x * 2
            OpCode::Return,
        ],
        1,
    );

    let main = Function::new(
        "main",
        0,
        vec![
            OpCode::Push(Value::Int(3)),
            OpCode::Call(2),   // double(3) → 6
            OpCode::Call(1),   // square(6) → 36
            OpCode::Halt,
        ],
        0,
    );

    let vm = run_program(vec![main, square, double]);
    assert_eq!(vm.stack, vec![Value::Int(36)]);
}
