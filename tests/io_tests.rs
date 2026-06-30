use std::io::Read;
use std::io::Write;

use pausible::function::Function;
use pausible::io::HandleId;
use pausible::io::{FileMode, IoHandle, IoStrategy};
use pausible::opcode::OpCode;
use pausible::snapshot::Snapshot;
use pausible::value::Value;
use pausible::vm::{ResumeError, VM};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn run_code(code: Vec<OpCode>) -> VM {
    let mut vm = VM::new();
    let func = Function::new("main", 0, code, 0);
    vm.add_function(func);
    vm.prepare(0).unwrap();
    vm.run().unwrap();
    vm
}

fn run_to_yield(code: Vec<OpCode>) -> VM {
    let mut vm = VM::new();
    let func = Function::new("main", 0, code, 0);
    vm.add_function(func);
    vm.prepare(0).unwrap();
    while vm.running {
        vm.step().unwrap();
    }
    assert!(!vm.running, "expected VM to have yielded");
    vm
}

fn make_temp_file(name: &str, content: &[u8]) -> String {
    let path = format!("/tmp/{name}");
    std::fs::write(&path, content).expect("write temp file");
    path
}

#[test]
fn v2_snapshot_with_io_roundtrip() {
    let mut vm = run_code(vec![OpCode::Push(Value::Int(1)), OpCode::Halt]);

    vm.create_handle(IoHandle::Timer {
        ms: 500,
        strategy: IoStrategy::Replay,
    });
    vm.create_handle(IoHandle::Stdin {
        buffer: vec![1, 2, 3],
    });

    let snap = vm.create_snapshot();
    assert_eq!(snap.header.version, 3);
    assert_eq!(snap.header.io_handle_count, 2);

    let tmp = "/tmp/pausible_v2_io_roundtrip.bin";
    snap.write_to_file(tmp).unwrap();
    let loaded = Snapshot::read_from_file(tmp).unwrap();
    assert_eq!(loaded.header.io_handle_count, 2);
}

// ---------------------------------------------------------------------------
// 3.6.1  snapshot & restore
// ---------------------------------------------------------------------------

#[test]
fn file_yield_seek_reconnect_resume() {
    let path = make_temp_file("pausible_seek_test.txt", b"ABCDEFGHIJKLMNOP");

    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    vm.create_handle(IoHandle::File {
        path: path.clone(),
        mode: FileMode::Read,
        position: 5,
        strategy: IoStrategy::Seek,
        file: None,
        cached: None,
    });

    let snap = vm.create_snapshot();
    assert_eq!(snap.header.io_handle_count, 1);

    let result = vm.resume(&snap);
    assert!(result.is_ok(), "resume should succeed: {result:?}");

    let handle = vm.get_handle(HandleId(0)).expect("handle should exist");
    if let IoHandle::File { file, position, .. } = handle {
        assert_eq!(*position, 5);
        if let Some(f) = file {
            let mut f = f.try_clone().expect("clone file");
            let mut buf = [0u8; 5];
            f.read_exact(&mut buf).unwrap();
            assert_eq!(&buf, b"FGHIJ");
        }
    } else {
        panic!("expected File handle");
    }
}

#[test]
fn ephemeral_stdin_cache_restore() {
    let mut vm = run_code(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    let stdin_handle = IoHandle::Stdin {
        buffer: b"cached_input".to_vec(),
    };
    let handle_id = vm.create_handle(stdin_handle);

    let snap = vm.create_snapshot();
    assert_eq!(snap.header.io_handle_count, 1);

    let result = vm.resume(&snap);
    assert!(result.is_ok(), "resume should succeed: {result:?}");

    let restored = vm.get_handle(handle_id).expect("handle should exist");
    if let IoHandle::Stdin { buffer } = restored {
        assert_eq!(buffer, b"cached_input");
    } else {
        panic!("expected Stdin handle");
    }
}

#[test]
fn multi_handle_mixed_strategies_snapshot() {
    let path = make_temp_file("pausible_multi_test.txt", b"multi-handle-content");

    let mut vm = run_code(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    vm.create_handle(IoHandle::File {
        path,
        mode: FileMode::Read,
        position: 10,
        strategy: IoStrategy::Seek,
        file: None,
        cached: None,
    });
    vm.create_handle(IoHandle::Stdin {
        buffer: b"stdin-data".to_vec(),
    });

    let snap = vm.create_snapshot();
    assert_eq!(snap.header.io_handle_count, 2);

    let result = vm.resume(&snap);
    assert!(
        result.is_ok(),
        "multi-handle resume should succeed: {result:?}"
    );
    assert_eq!(vm.handles.len(), 2);
}

// ---------------------------------------------------------------------------
// 3.6.2  error paths
// ---------------------------------------------------------------------------

#[test]
fn file_deleted_before_resume_resource_lost() {
    let path = make_temp_file("pausible_delete_test.txt", b"delete-me");

    let mut vm = run_code(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    vm.create_handle(IoHandle::File {
        path: path.clone(),
        mode: FileMode::Read,
        position: 0,
        strategy: IoStrategy::Seek,
        file: None,
        cached: None,
    });

    let snap = vm.create_snapshot();
    std::fs::remove_file(&path).unwrap();

    let result = vm.resume(&snap);
    match result {
        Err(ResumeError::Reconnect(msg)) => {
            assert!(msg.contains("Failed"), "expected Failed in: {msg}");
        }
        other => panic!("expected Reconnect error, got {other:?}"),
    }
}

/// POSIX allows seeking beyond EOF, so truncation does NOT cause a
/// reconnect failure.  The position is still correctly restored.
#[test]
fn file_truncated_seek_position_preserved() {
    let path = make_temp_file("pausible_trunc_test.txt", &[b'A'; 200]);

    let mut vm = run_code(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    vm.create_handle(IoHandle::File {
        path: path.clone(),
        mode: FileMode::Read,
        position: 150,
        strategy: IoStrategy::Seek,
        file: None,
        cached: None,
    });

    let snap = vm.create_snapshot();

    // Truncate to 50 bytes — seek to 150 is still valid in POSIX.
    std::fs::write(&path, [b'B'; 50]).unwrap();

    let result = vm.resume(&snap);
    assert!(
        result.is_ok(),
        "seek beyond EOF is valid in POSIX: {result:?}"
    );

    let handle = vm.get_handle(HandleId(0)).expect("handle should exist");
    if let IoHandle::File { position, .. } = handle {
        assert_eq!(*position, 150);
    }
}

// ---------------------------------------------------------------------------
// TCP  tests
// ---------------------------------------------------------------------------

fn spawn_echo_server() -> (std::thread::JoinHandle<()>, u16) {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind echo server");
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        for mut s in listener.incoming().take(1).flatten() {
            let mut buf = [0u8; 1024];
            if let Ok(n) = s.read(&mut buf) {
                let _ = s.write_all(&buf[..n]);
            }
        }
    });
    (handle, port)
}

#[test]
fn tcp_yield_reconnect_echo() {
    let (_jh, port) = spawn_echo_server();
    let addr = format!("127.0.0.1:{port}");

    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    let stream = std::net::TcpStream::connect(&addr).expect("connect");
    let handle_id = vm.create_handle(IoHandle::TcpStream {
        addr: addr.clone(),
        strategy: IoStrategy::Replay,
        stream: Some(stream),
        last_request: None,
        last_response: None,
    });

    let snap = vm.create_snapshot();
    assert_eq!(snap.header.io_handle_count, 1);

    let result = vm.resume(&snap);
    assert!(
        result.is_ok(),
        "TCP reconnect resume should succeed: {result:?}"
    );

    let handle = vm.get_handle(handle_id).expect("handle should exist");
    assert!(matches!(handle, IoHandle::TcpStream { .. }));
}

/// TCP reconnect fails when the peer port has nothing listening.
#[test]
fn tcp_peer_closed_reconnect_fails() {
    // Use a port that has nothing listening on it.
    // We bind then immediately drop to find a free port, but then
    // never start a listener on it again.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().unwrap().port()
    };
    let addr = format!("127.0.0.1:{port}");

    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    vm.create_handle(IoHandle::TcpStream {
        addr: addr.clone(),
        strategy: IoStrategy::Replay,
        stream: None,
        last_request: None,
        last_response: None,
    });

    let snap = vm.create_snapshot();

    // Nothing is listening on this port after we dropped the listener.
    let result = vm.resume(&snap);
    match result {
        Err(ResumeError::Reconnect(msg)) => {
            assert!(msg.contains("Failed"), "expected Failed in: {msg}");
        }
        other => panic!("expected Reconnect error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// HTTP  tests  (local mock server)
// ---------------------------------------------------------------------------

/// Spawn a minimal HTTP server that returns `response_body` for every GET.
fn spawn_fixed_http_server(response_body: &'static str) -> (std::thread::JoinHandle<()>, String) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/fixed");
    let body = response_body.to_string();
    let handle = std::thread::spawn(move || {
        for mut s in listener.incoming().take(10).flatten() {
            let mut buf = [0u8; 4096];
            if s.read(&mut buf).unwrap_or(0) == 0 {
                continue;
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(response.as_bytes());
        }
    });
    (handle, url)
}

/// Spawn a minimal HTTP server that returns an incrementing counter.
fn spawn_counting_http_server() -> (std::thread::JoinHandle<()>, String) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/counter");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let handle = std::thread::spawn(move || {
        for mut s in listener.incoming().take(10).flatten() {
            let mut buf = [0u8; 4096];
            if s.read(&mut buf).unwrap_or(0) == 0 {
                continue;
            }
            let n = c.fetch_add(1, Ordering::SeqCst);
            let body = format!("counter: {n}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(response.as_bytes());
        }
    });
    (handle, url)
}

/// HTTP GET replay: response matches after snapshot+resume.
#[test]
fn http_get_replay_response_matches() {
    let (_jh, url) = spawn_fixed_http_server("fixed-body");

    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    vm.create_handle(IoHandle::HttpConnection {
        url: url.clone(),
        method: pausible::io::HttpMethod::Get,
        body: None,
        last_response: Some(b"fixed-body".to_vec()),
        strategy: IoStrategy::Replay,
    });

    let snap = vm.create_snapshot();
    assert_eq!(snap.header.io_handle_count, 1);

    let result = vm.resume(&snap);
    assert!(result.is_ok(), "HTTP GET replay should succeed: {result:?}");
}

/// HTTP POST replay: request body is re-sent after resume.
#[test]
fn http_post_replay() {
    let (_jh, url) = spawn_fixed_http_server("echo-back-post");

    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    let body_str = "hello=world";

    vm.create_handle(IoHandle::HttpConnection {
        url: url.clone(),
        method: pausible::io::HttpMethod::Post,
        body: Some(body_str.as_bytes().to_vec()),
        last_response: Some(b"echo-back-post".to_vec()),
        strategy: IoStrategy::Replay,
    });

    let snap = vm.create_snapshot();
    assert_eq!(snap.header.io_handle_count, 1);

    let result = vm.resume(&snap);
    assert!(
        result.is_ok(),
        "HTTP POST replay should succeed: {result:?}"
    );
}

/// HTTP GET to counter endpoint: each response differs, so replay
/// detects `DataDiverged` via the `ReconnectReport`.
#[test]
fn http_get_replay_data_diverged() {
    let (_jh, url) = spawn_counting_http_server();

    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    vm.create_handle(IoHandle::HttpConnection {
        url: url.clone(),
        method: pausible::io::HttpMethod::Get,
        body: None,
        // Store a non-matching fake response so replay will diverge.
        last_response: Some(b"old-value-that-wont-match".to_vec()),
        strategy: IoStrategy::Replay,
    });

    let snap = vm.create_snapshot();
    let report = snap.restore_io_handles(&mut vm);
    let degraded: Vec<_> = report
        .entries
        .iter()
        .filter(|(_, s)| matches!(s, pausible::snapshot::ReconnectStatus::Degraded { .. }))
        .collect();
    assert!(
        !degraded.is_empty(),
        "expected DataDiverged (Degraded) for changing endpoint replay"
    );
}

/// HTTP endpoint unreachable: replay fails, reports Failed.
#[test]
fn http_endpoint_unreachable_reconnect_fails() {
    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    vm.create_handle(IoHandle::HttpConnection {
        url: "http://127.0.0.1:1/nonexistent".to_string(),
        method: pausible::io::HttpMethod::Get,
        body: None,
        last_response: Some(b"old".to_vec()),
        strategy: IoStrategy::Replay,
    });

    let snap = vm.create_snapshot();
    let report = snap.restore_io_handles(&mut vm);
    assert!(
        report.has_failures(),
        "expected failures for unreachable HTTP endpoint"
    );
}

// -----------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 4.6  Concurrent I/O  (Phase 4)
// ---------------------------------------------------------------------------

/// One child task does `FileOpen` + `FileRead`, parent waits then yields.
/// After resume, verify the child's return value is preserved and the
/// child's per-task I/O handle is captured in the snapshot.
#[test]
fn child_file_read_yield_resume_preserves_return_value() {
    let path = make_temp_file("pausible_child_io.txt", b"ChildIOData");

    let mut vm = VM::new();

    // Allocate heap strings for file paths so they survive Function::new().
    let path_gc = vm.alloc_string(path.clone());
    let mode_gc = vm.alloc_string("r".into());

    // Child: open file, read, close, push content, return.
    let child_fn = Function::new(
        "child_io",
        0,
        vec![
            OpCode::FileOpen {
                path: Value::String(path_gc),
                mode: Value::String(mode_gc),
            },
            OpCode::FileRead(HandleId(0)),
            OpCode::Return,
        ],
        0,
    );

    // Main: spawn child, wait, yield, halt.
    let main_fn = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1),     // child
            OpCode::WaitChildren, // collect return value
            OpCode::Yield,        // snapshot + pause
            OpCode::Halt,
        ],
        0,
    );

    vm.add_function(main_fn);
    vm.add_function(child_fn);
    vm.prepare(0).unwrap();
    vm.run().unwrap();

    // After yield, parent has the child's return value on stack.
    assert_eq!(vm.stack.len(), 1);
    assert!(
        matches!(vm.stack[0], Value::List(_)),
        "return value should be a List (file content bytes)"
    );

    let snap = vm.snapshot.clone().expect("snapshot should exist");

    // Resume into a fresh VM.
    let mut resumed = VM::new();
    let rpath_gc = resumed.alloc_string(path.clone());
    let rmode_gc = resumed.alloc_string("r".into());
    let resumed_child = Function::new(
        "child_io",
        0,
        vec![
            OpCode::FileOpen {
                path: Value::String(rpath_gc),
                mode: Value::String(rmode_gc),
            },
            OpCode::FileRead(HandleId(0)),
            OpCode::Return,
        ],
        0,
    );
    let resumed_main = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1),
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );
    resumed.add_function(resumed_main);
    resumed.add_function(resumed_child);
    resumed.prepare(0).unwrap();

    let result = resumed.resume(&snap);
    assert!(result.is_ok(), "resume should succeed: {result:?}");

    // After resume, stack should still have the child's file content.
    assert_eq!(resumed.stack.len(), 1);
    assert!(
        matches!(resumed.stack[0], Value::List(_)),
        "resumed stack should preserve file content list"
    );

    // Child task should be in registry with Completed status.
    assert_eq!(resumed.task_count(), 2);
    let child = resumed
        .task_registry
        .get(&pausible::task::TaskId(1))
        .expect("child task should exist");
    assert_eq!(child.status, pausible::task::TaskStatus::Completed);
}

/// Verify per-task I/O handles are properly isolated across multiple
/// completed child tasks in the snapshot.
#[test]
fn per_task_io_handle_isolation_in_snapshot() {
    let path_a = make_temp_file("pausible_iso_a.txt", b"AAAA");
    let path_b = make_temp_file("pausible_iso_b.txt", b"BBBB");

    // Create a VM that runs a simple yield point.
    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    // Create child task 1 with a File handle.
    let child_id_1 = pausible::task::TaskId(1);
    let mut task_a = pausible::task::Task::new(child_id_1, Some(pausible::task::TaskId::root()));
    task_a.status = pausible::task::TaskStatus::Completed;
    task_a.io_handles.insert(
        HandleId(100),
        IoHandle::File {
            path: path_a.clone(),
            mode: FileMode::Read,
            position: 2,
            strategy: IoStrategy::Seek,
            file: None,
            cached: Some(b"AAAA".to_vec()),
        },
    );
    task_a.stack.push(Value::Int(1));
    vm.task_registry.insert(child_id_1, task_a);

    // Create child task 2 with a File handle.
    let child_id_2 = pausible::task::TaskId(2);
    let mut task_b = pausible::task::Task::new(child_id_2, Some(pausible::task::TaskId::root()));
    task_b.status = pausible::task::TaskStatus::Completed;
    task_b.io_handles.insert(
        HandleId(200),
        IoHandle::File {
            path: path_b.clone(),
            mode: FileMode::Read,
            position: 0,
            strategy: IoStrategy::Seek,
            file: None,
            cached: Some(b"BBBB".to_vec()),
        },
    );
    task_b.stack.push(Value::Int(2));
    vm.task_registry.insert(child_id_2, task_b);

    // Update root's children list.
    vm.current_task_mut().children = vec![child_id_1, child_id_2];

    // Capture snapshot.
    let snap = vm.create_snapshot();
    assert_eq!(
        snap.header.task_count, 3,
        "should capture root + 2 children"
    );

    // Write to file and read back to verify binary format.
    let tmp = "/tmp/pausible_iso_snap.bin";
    snap.write_to_file(tmp).unwrap();
    let loaded = Snapshot::read_from_file(tmp).unwrap();
    assert_eq!(loaded.header.version, 3);
    assert_eq!(loaded.header.task_count, 3);

    // Resume into a fresh VM.
    let mut resumed = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);
    resumed.current_task_mut().children = vec![child_id_1, child_id_2];

    let result = resumed.resume(&loaded);
    assert!(result.is_ok(), "resume should succeed: {result:?}");

    // Both children should have their per-task I/O handles restored.
    for (child_id, expected_hid) in &[(child_id_1, HandleId(100)), (child_id_2, HandleId(200))] {
        let task = resumed
            .task_registry
            .get(child_id)
            .expect("child task should exist");
        assert_eq!(
            task.io_handles.len(),
            1,
            "child {child_id:?} should have 1 I/O handle"
        );
        assert!(
            task.io_handles.contains_key(expected_hid),
            "child {child_id:?} should contain {expected_hid:?}"
        );
    }

    // Root should have empty io_handles (parent had no I/O).
    let root = resumed
        .task_registry
        .get(&pausible::task::TaskId::root())
        .expect("root should exist");
    assert!(
        root.io_handles.is_empty(),
        "root should have no I/O handles"
    );
}

/// Snapshot file roundtrip for a task tree with per-task I/O handles.
#[test]
fn task_tree_with_io_snapshot_file_roundtrip() {
    let path = make_temp_file("pausible_roundtrip_io.txt", b"RoundTripData");

    let mut vm = VM::new();
    let path_gc = vm.alloc_string(path.clone());
    let mode_gc = vm.alloc_string("r".into());

    let child_fn = Function::new(
        "child",
        0,
        vec![
            OpCode::FileOpen {
                path: Value::String(path_gc),
                mode: Value::String(mode_gc),
            },
            OpCode::FileRead(HandleId(0)),
            OpCode::Return,
        ],
        0,
    );

    let main_fn = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1),
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );

    vm.add_function(main_fn);
    vm.add_function(child_fn);
    vm.prepare(0).unwrap();
    vm.run().unwrap();

    let snap = vm.snapshot.take().unwrap();

    let tmp = "/tmp/pausible_4_6_snap.bin";
    snap.write_to_file(tmp).unwrap();
    let loaded = Snapshot::read_from_file(tmp).unwrap();

    assert_eq!(loaded.header.version, 3);
    assert_eq!(loaded.header.task_count, snap.header.task_count);

    // Resume from the loaded snapshot.
    let mut rest_vm = VM::new();
    let rpath_gc = rest_vm.alloc_string(path.clone());
    let rmode_gc = rest_vm.alloc_string("r".into());
    let rchild = Function::new(
        "child",
        0,
        vec![
            OpCode::FileOpen {
                path: Value::String(rpath_gc),
                mode: Value::String(rmode_gc),
            },
            OpCode::FileRead(HandleId(0)),
            OpCode::Return,
        ],
        0,
    );
    let rmain = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1),
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );
    rest_vm.add_function(rmain);
    rest_vm.add_function(rchild);
    rest_vm.prepare(0).unwrap();

    let result = rest_vm.resume(&loaded);
    assert!(result.is_ok(), "resume should succeed: {result:?}");
    assert_eq!(rest_vm.stack.len(), 1);
    assert_eq!(rest_vm.task_count(), 2);
}
/// Verify that a snapshot containing per-task I/O handles
/// (including Cached strategy handles) serializes correctly to binary
/// format and can be read back with all task data intact.
#[test]
fn cached_io_handle_in_task_snapshot_binary_format() {
    let path = make_temp_file("pausible_cached_snap.txt", b"CachedSnapData");

    let mut vm = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);

    // Create a child task with a Cached file handle.
    let child_id = pausible::task::TaskId(1);
    let mut child = pausible::task::Task::new(child_id, Some(pausible::task::TaskId::root()));
    child.status = pausible::task::TaskStatus::Completed;
    child.io_handles.insert(
        HandleId(0),
        IoHandle::File {
            path: path.clone(),
            mode: FileMode::Read,
            position: 0,
            strategy: IoStrategy::Cached,
            file: None,
            cached: Some(b"cached-in-snapshot".to_vec()),
        },
    );
    child.stack.push(Value::Int(42));
    vm.task_registry.insert(child_id, child);
    vm.current_task_mut().children = vec![child_id];

    // Create snapshot with task tree.
    let snap = vm.create_snapshot();
    assert_eq!(
        snap.header.task_count, 2,
        "should contain root (yielded) + child (completed)"
    );

    // Write to file, read back, verify task count.
    let tmp = "/tmp/pausible_cached_task_snap.bin";
    snap.write_to_file(tmp).unwrap();
    let loaded = Snapshot::read_from_file(tmp).unwrap();
    assert_eq!(loaded.header.version, 3);
    assert_eq!(loaded.header.task_count, 2);

    // Delete the original file — cached data is in the snapshot, not on disk.
    let _ = std::fs::remove_file(
        std::path::Path::new(&path)
            .parent()
            .unwrap_or(std::path::Path::new("/")),
    );

    // Restore task tree from loaded snapshot.
    let mut resumed = run_to_yield(vec![
        OpCode::Push(Value::Int(0)),
        OpCode::Yield,
        OpCode::Halt,
    ]);
    resumed.current_task_mut().children = vec![child_id];

    let result = snap.restore_task_tree(&mut resumed);
    assert!(
        result.is_ok(),
        "restore_task_tree from binary file should succeed: {result:?}"
    );

    // Child should have the Cached handle with cached data.
    let child = resumed
        .task_registry
        .get(&child_id)
        .expect("child should exist after restore");
    assert_eq!(child.io_handles.len(), 1);
    let h = child.io_handles.get(&HandleId(0)).unwrap();
    assert_eq!(h.strategy(), IoStrategy::Cached);
    assert_eq!(child.stack, vec![Value::Int(42)]);
}

/// Verify that a child task that performs I/O (`FileOpen` + `FileRead`)
/// through the actual `WaitChildren` execution path preserves its I/O
/// handles in the snapshot, so they survive yield → resume.
#[test]
fn child_io_handles_survive_wait_yield_resume() {
    let tmp = std::env::temp_dir().join("pausible_child_handles_survive.bin");
    let path = tmp.to_str().unwrap();
    std::fs::write(path, b"HandleSurviveData").unwrap();

    let mut vm = VM::new();
    let path_gc = vm.alloc_string(path.to_string());
    let mode_gc = vm.alloc_string("r".into());

    // Child: open file, read it, return the content.
    let child_fn = Function::new(
        "child",
        0,
        vec![
            OpCode::FileOpen {
                path: Value::String(path_gc),
                mode: Value::String(mode_gc),
            },
            OpCode::FileRead(HandleId(0)),
            OpCode::Return,
        ],
        0,
    );

    // Main: spawn child, wait, yield, halt.
    let main_fn = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1),
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );

    vm.add_function(main_fn);
    vm.add_function(child_fn);
    vm.prepare(0).unwrap();
    vm.run().unwrap();

    let snap = vm.snapshot.clone().expect("snapshot should exist");

    // Write to file to exercise serialisation path.
    let snap_path = tmp.parent().unwrap().join("pausible_survive_snap.bin");
    snap.write_to_file(snap_path.to_str().unwrap()).unwrap();
    let loaded = Snapshot::read_from_file(snap_path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(snap_path);

    // Resume into a fresh VM.
    let mut resumed = VM::new();
    let rpath_gc = resumed.alloc_string(path.to_string());
    let rmode_gc = resumed.alloc_string("r".into());
    let rchild = Function::new(
        "child",
        0,
        vec![
            OpCode::FileOpen {
                path: Value::String(rpath_gc),
                mode: Value::String(rmode_gc),
            },
            OpCode::FileRead(HandleId(0)),
            OpCode::Return,
        ],
        0,
    );
    let rmain = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1),
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );
    resumed.add_function(rmain);
    resumed.add_function(rchild);
    resumed.prepare(0).unwrap();

    let result = resumed.resume(&loaded);
    assert!(result.is_ok(), "resume should succeed: {result:?}");

    // Verify the child task has its I/O handles preserved.
    let child_id = pausible::task::TaskId(1);
    let child = resumed
        .task_registry
        .get(&child_id)
        .expect("child task should exist after resume");
    assert_eq!(child.status, pausible::task::TaskStatus::Completed);
    assert!(
        !child.io_handles.is_empty(),
        "child should have I/O handles preserved after resume,          got empty io_handles"
    );
    assert!(
        child.io_handles.contains_key(&HandleId(0)),
        "child should contain HandleId(0) from FileOpen"
    );

    // Also verify the child's stack was preserved.
    assert!(
        !child.stack.is_empty(),
        "child stack should be preserved in registry"
    );

    // Clean up.
    let _ = std::fs::remove_file(path);
}

// ===========================================================================
// 4.6 并发 I/O — remaining tests
// ===========================================================================

/// Parent spawns two child tasks that each do HTTP GET to two different
/// local servers, waits for both to complete, then yields. After snapshot
/// capture and resume (with the servers still running), the Replay
/// strategy reconnects both HTTP handles successfully.
#[test]
#[allow(clippy::too_many_lines, clippy::similar_names)]
fn concurrent_http_children_yield_resume() {
    let (_jh_a, url_a) = spawn_fixed_http_server("response-A");
    let (_jh_b, url_b) = spawn_fixed_http_server("response-B");

    let mut vm = VM::new();
    let url_a_gc = vm.alloc_string(url_a.clone());
    let url_b_gc = vm.alloc_string(url_b.clone());

    // Child A: HTTP GET url_a → Return
    let child_a_fn = Function::new(
        "child_a",
        0,
        vec![
            OpCode::HttpGet {
                url: Value::String(url_a_gc),
            },
            OpCode::Return,
        ],
        0,
    );

    // Child B: HTTP GET url_b → Return
    let child_b_fn = Function::new(
        "child_b",
        0,
        vec![
            OpCode::HttpGet {
                url: Value::String(url_b_gc),
            },
            OpCode::Return,
        ],
        0,
    );

    // Main: spawn child_a, spawn child_b, wait, yield, halt
    let main_fn = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1), // child_a
            OpCode::Spawn(2), // child_b
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );

    vm.add_function(main_fn);
    vm.add_function(child_a_fn);
    vm.add_function(child_b_fn);
    vm.prepare(0).unwrap();
    vm.run().unwrap();

    // After yield, parent has both return values on stack.
    assert_eq!(vm.stack.len(), 2);
    assert!(
        matches!(vm.stack[0], Value::String(_)),
        "first return value should be String (HTTP body)"
    );
    assert!(
        matches!(vm.stack[1], Value::String(_)),
        "second return value should be String (HTTP body)"
    );

    let snap = vm.snapshot.clone().expect("snapshot should exist");

    // Resume into a fresh VM (HTTP servers are still running).
    let mut resumed = VM::new();
    let r_url_a = resumed.alloc_string(url_a.clone());
    let r_url_b = resumed.alloc_string(url_b.clone());

    let r_child_a = Function::new(
        "child_a",
        0,
        vec![
            OpCode::HttpGet {
                url: Value::String(r_url_a),
            },
            OpCode::Return,
        ],
        0,
    );
    let r_child_b = Function::new(
        "child_b",
        0,
        vec![
            OpCode::HttpGet {
                url: Value::String(r_url_b),
            },
            OpCode::Return,
        ],
        0,
    );
    let r_main = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1),
            OpCode::Spawn(2),
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );

    resumed.add_function(r_main);
    resumed.add_function(r_child_a);
    resumed.add_function(r_child_b);
    resumed.prepare(0).unwrap();

    let result = resumed.resume(&snap);
    assert!(result.is_ok(), "resume should succeed: {result:?}");

    // Stack should still have both return values.
    assert_eq!(resumed.stack.len(), 2);

    // Both children should have their I/O handles preserved in registry.
    for cid in [pausible::task::TaskId(1), pausible::task::TaskId(2)] {
        let child = resumed
            .task_registry
            .get(&cid)
            .expect("child should exist after resume");
        assert_eq!(child.status, pausible::task::TaskStatus::Completed);
        assert!(
            !child.io_handles.is_empty(),
            "child should have HTTP I/O handle"
        );
        let has_http = child
            .io_handles
            .values()
            .any(|h| matches!(h, IoHandle::HttpConnection { .. }));
        assert!(has_http, "child should have HttpConnection handle");
    }

    // Child stacks are empty after return (values were popped
    // into the parent stack by WaitChildren). This is correct behavior:
    // return values transfer from child to parent.
    for cid in [pausible::task::TaskId(1), pausible::task::TaskId(2)] {
        let child = resumed.task_registry.get(&cid).unwrap();
        assert!(
            child.stack.is_empty(),
            "child stack should be empty after return value moved to parent"
        );
    }
}

/// Three child tasks with different I/O strategies run concurrently:
/// child A: `FileOpen` + `FileRead` (Seek), child B: `HttpGet` (Replay),
/// child C: `StdinRead` (Cached). Parent waits for all, yields, snapshot
/// → resume. Verify all three I/O types reconnect correctly.
#[test]
#[allow(clippy::too_many_lines, clippy::similar_names)]
fn mixed_io_types_concurrent_yield_resume() {
    let path = make_temp_file("pausible_mixed_io.txt", b"MixedIOData");
    let (_jh, url) = spawn_fixed_http_server("http-mixed-response");

    let mut vm = VM::new();
    let path_gc = vm.alloc_string(path.clone());
    let mode_gc = vm.alloc_string("r".into());
    let url_gc = vm.alloc_string(url.clone());

    // Child A: FileOpen + FileRead + Return
    let child_a_fn = Function::new(
        "child_a",
        0,
        vec![
            OpCode::FileOpen {
                path: Value::String(path_gc),
                mode: Value::String(mode_gc),
            },
            OpCode::FileRead(HandleId(0)),
            OpCode::Return,
        ],
        0,
    );

    // Child B: HttpGet + Return
    let child_b_fn = Function::new(
        "child_b",
        0,
        vec![
            OpCode::HttpGet {
                url: Value::String(url_gc),
            },
            OpCode::Return,
        ],
        0,
    );

    // Child C: StdinRead + Return
    let child_c_fn = Function::new("child_c", 0, vec![OpCode::StdinRead, OpCode::Return], 0);

    // Main: spawn A, B, C, wait, yield, halt
    let main_fn = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1), // child_a (File)
            OpCode::Spawn(2), // child_b (HTTP)
            OpCode::Spawn(3), // child_c (Stdin)
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );

    vm.add_function(main_fn);
    vm.add_function(child_a_fn);
    vm.add_function(child_b_fn);
    vm.add_function(child_c_fn);
    vm.prepare(0).unwrap();

    // Step through the three Spawn ops so we can inject the Stdin
    // handle into child C before WaitChildren executes it.
    vm.step().unwrap(); // Spawn(1)
    vm.step().unwrap(); // Spawn(2)
    vm.step().unwrap(); // Spawn(3)

    // Inject Stdin handle into child C's task.
    if let Some(child_c) = vm.task_registry.get_mut(&pausible::task::TaskId(3)) {
        child_c.io_handles.insert(
            HandleId(0),
            IoHandle::Stdin {
                buffer: vec![65, 66, 67], // "ABC"
            },
        );
    }

    // Continue execution: WaitChildren → Yield → stop.
    while vm.running {
        vm.step().unwrap();
    }
    assert!(!vm.running, "expected VM to have yielded");

    // After yield, parent has 3 return values on stack.
    assert_eq!(vm.stack.len(), 3);

    let snap = vm.snapshot.clone().expect("snapshot should exist");

    // ----- Resume into a fresh VM -----
    let mut resumed = VM::new();
    let r_path = resumed.alloc_string(path.clone());
    let r_mode = resumed.alloc_string("r".into());
    let r_url = resumed.alloc_string(url.clone());

    let r_a = Function::new(
        "child_a",
        0,
        vec![
            OpCode::FileOpen {
                path: Value::String(r_path),
                mode: Value::String(r_mode),
            },
            OpCode::FileRead(HandleId(0)),
            OpCode::Return,
        ],
        0,
    );
    let r_b = Function::new(
        "child_b",
        0,
        vec![
            OpCode::HttpGet {
                url: Value::String(r_url),
            },
            OpCode::Return,
        ],
        0,
    );
    let r_c = Function::new("child_c", 0, vec![OpCode::StdinRead, OpCode::Return], 0);
    let r_main = Function::new(
        "main",
        0,
        vec![
            OpCode::Spawn(1),
            OpCode::Spawn(2),
            OpCode::Spawn(3),
            OpCode::WaitChildren,
            OpCode::Yield,
            OpCode::Halt,
        ],
        0,
    );

    resumed.add_function(r_main);
    resumed.add_function(r_a);
    resumed.add_function(r_b);
    resumed.add_function(r_c);
    resumed.prepare(0).unwrap();

    let result = resumed.resume(&snap);
    assert!(result.is_ok(), "resume should succeed: {result:?}");

    assert_eq!(resumed.stack.len(), 3);
    assert_eq!(resumed.task_count(), 4); // root + 3 children

    // Verify all three children are preserved with correct status.
    for (cid, expected_kind) in &[
        (pausible::task::TaskId(1), "File"),
        (pausible::task::TaskId(2), "HttpConnection"),
        (pausible::task::TaskId(3), "Stdin"),
    ] {
        let child = resumed
            .task_registry
            .get(cid)
            .expect("child should exist after resume");
        assert_eq!(
            child.status,
            pausible::task::TaskStatus::Completed,
            "child should be Completed"
        );
        assert!(!child.io_handles.is_empty(), "child should have I/O handle");
        let has_kind = child
            .io_handles
            .values()
            .any(|h| h.kind_name() == *expected_kind);
        assert!(
            has_kind,
            "child should have {expected_kind} handle, got: {:?}",
            child.io_handles.keys().collect::<Vec<_>>()
        );
    }
}
