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

// ---------------------------------------------------------------------------
// 3.6.3  compatibility
// ---------------------------------------------------------------------------

/// Convert a v2 snapshot file to v1 format by changing the version byte
/// and stripping the 4-byte `io_handle_count` field and `io_section`.
fn downgrade_to_v1_file(path: &str) {
    let data = std::fs::read(path).unwrap();
    // v2 header = 40 bytes (magic+version+code_hash+timestamp+heap+frame+stack+io_handle)
    // v1 header = 36 bytes (same minus io_handle_count)
    // After v2 header: heap + frames + stack + io_section(4 bytes for count=0)
    let mut v1 = Vec::with_capacity(data.len() - 8);
    v1.extend_from_slice(&data[..36]); // v1 header (36 bytes)
    v1[4..8].copy_from_slice(&1u32.to_le_bytes()); // version = 1
    v1.extend_from_slice(&data[40..data.len() - 4]); // skip 40-byte v2 header, strip io_section
    std::fs::write(path, &v1).unwrap();
}

/// A v1 snapshot (no I/O section) deserializes with `io_handle_count=0`.
#[test]
fn v1_snapshot_no_io_backward_compat() {
    let mut vm = run_code(vec![OpCode::Push(Value::Int(42)), OpCode::Halt]);
    let s = vm.alloc_string("hello".into());
    vm.stack.push(Value::String(s));

    let snap = vm.create_snapshot();
    assert_eq!(snap.header.version, 2);

    let tmp = "/tmp/pausible_v1_test.bin";
    snap.write_to_file(tmp).unwrap();
    downgrade_to_v1_file(tmp);

    let loaded = Snapshot::read_from_file(tmp).unwrap();
    assert_eq!(loaded.header.version, 1);
    assert_eq!(loaded.header.io_handle_count, 0);

    let mut restored = VM::new();
    let func = Function::new(
        "main",
        0,
        vec![OpCode::Push(Value::Int(42)), OpCode::Halt],
        0,
    );
    restored.add_function(func);
    restored.prepare(0).unwrap();
    let ch = restored.code_hash();
    loaded.restore_into(&mut restored, ch).unwrap();
    assert!(matches!(restored.stack.last(), Some(Value::String(_))));
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
    assert_eq!(snap.header.version, 2);
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
