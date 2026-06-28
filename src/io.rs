use core::fmt;
use std::fs::File;
use std::net::TcpStream;

// -- IoStrategy --

/// How an I/O handle behaves when the program yields and later resumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoStrategy {
    /// Re-issue the original request; trigger `DataDiverged` if the result differs.
    Replay,
    /// Re-open the resource and seek to the recorded position.
    Seek,
    /// Use the snapshot-cached data; no reconnection is attempted.
    Cached,
}

impl fmt::Display for IoStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Replay => write!(f, "Replay"),
            Self::Seek => write!(f, "Seek"),
            Self::Cached => write!(f, "Cached"),
        }
    }
}

// -- HandleId --

/// Opaque identifier for a registered I/O handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleId(pub u32);

impl fmt::Display for HandleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

// -- FileMode / HttpMethod --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Read,
    Write,
    Append,
}

impl fmt::Display for FileMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => write!(f, "r"),
            Self::Write => write!(f, "w"),
            Self::Append => write!(f, "a"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Get => write!(f, "GET"),
            Self::Post => write!(f, "POST"),
        }
    }
}

// -- IoHandle --

/// A registered I/O handle in the VM.
///
/// Each variant carries the state needed to snapshot and reconnect.
/// Standard streams (Stdin / Stdout / Stderr) are implicitly `Cached`.
#[derive(Debug)]
pub enum IoHandle {
    File {
        path: String,
        mode: FileMode,
        position: u64,
        strategy: IoStrategy,
        file: Option<File>,
        /// Cached data for `Cached` strategy; also stores last-read snapshot for all strategies.
        cached: Option<Vec<u8>>,
    },
    TcpStream {
        addr: String,
        strategy: IoStrategy,
        stream: Option<TcpStream>,
        last_request: Option<Vec<u8>>,
        last_response: Option<Vec<u8>>,
    },
    HttpConnection {
        url: String,
        method: HttpMethod,
        body: Option<Vec<u8>>,
        last_response: Option<Vec<u8>>,
        strategy: IoStrategy,
    },
    Timer {
        ms: u64,
        strategy: IoStrategy,
    },
    Stdin {
        buffer: Vec<u8>,
    },
    Stdout {
        buffer: Vec<u8>,
    },
    Stderr {
        buffer: Vec<u8>,
    },
}

impl IoHandle {
    /// The strategy for this handle. Standard streams are always `Cached`.
    #[must_use]
    pub fn strategy(&self) -> IoStrategy {
        match self {
            Self::File { strategy, .. }
            | Self::TcpStream { strategy, .. }
            | Self::HttpConnection { strategy, .. }
            | Self::Timer { strategy, .. } => *strategy,
            Self::Stdin { .. } | Self::Stdout { .. } | Self::Stderr { .. } => IoStrategy::Cached,
        }
    }

    /// Human-readable kind label for error messages and reports.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::File { .. } => "File",
            Self::TcpStream { .. } => "TcpStream",
            Self::HttpConnection { .. } => "HttpConnection",
            Self::Timer { .. } => "Timer",
            Self::Stdin { .. } => "Stdin",
            Self::Stdout { .. } => "Stdout",
            Self::Stderr { .. } => "Stderr",
        }
    }
}


impl Clone for IoHandle {
    fn clone(&self) -> Self {
        match self {
            Self::File { path, mode, position, strategy, cached, .. } => Self::File {
                path: path.clone(),
                mode: *mode,
                position: *position,
                strategy: *strategy,
                file: None,
                cached: cached.clone(),
            },
            Self::TcpStream { addr, strategy, last_request, last_response, .. } => Self::TcpStream {
                addr: addr.clone(),
                strategy: *strategy,
                stream: None,
                last_request: last_request.clone(),
                last_response: last_response.clone(),
            },
            Self::HttpConnection { url, method, body, last_response, strategy } => Self::HttpConnection {
                url: url.clone(),
                method: *method,
                body: body.clone(),
                last_response: last_response.clone(),
                strategy: *strategy,
            },
            Self::Timer { ms, strategy } => Self::Timer { ms: *ms, strategy: *strategy },
            Self::Stdin { buffer } => Self::Stdin { buffer: buffer.clone() },
            Self::Stdout { buffer } => Self::Stdout { buffer: buffer.clone() },
            Self::Stderr { buffer } => Self::Stderr { buffer: buffer.clone() },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_display() {
        assert_eq!(format!("{}", IoStrategy::Replay), "Replay");
        assert_eq!(format!("{}", IoStrategy::Seek), "Seek");
        assert_eq!(format!("{}", IoStrategy::Cached), "Cached");
    }

    #[test]
    fn handle_id_display() {
        assert_eq!(format!("{}", HandleId(0)), "#0");
        assert_eq!(format!("{}", HandleId(42)), "#42");
    }

    #[test]
    fn handle_id_equality() {
        assert_eq!(HandleId(1), HandleId(1));
        assert_ne!(HandleId(1), HandleId(2));
    }

    #[test]
    fn file_mode_display() {
        assert_eq!(format!("{}", FileMode::Read), "r");
        assert_eq!(format!("{}", FileMode::Write), "w");
        assert_eq!(format!("{}", FileMode::Append), "a");
    }

    #[test]
    fn http_method_display() {
        assert_eq!(format!("{}", HttpMethod::Get), "GET");
        assert_eq!(format!("{}", HttpMethod::Post), "POST");
    }

    #[test]
    fn file_handle_strategy() {
        let h = IoHandle::File {
            path: "/tmp/a.txt".into(),
            mode: FileMode::Read,
            position: 0,
            strategy: IoStrategy::Seek,
                file: None,
                cached: None,
        };
        assert_eq!(h.strategy(), IoStrategy::Seek);
        assert_eq!(h.kind_name(), "File");
    }

    #[test]
    fn stdin_is_always_cached() {
        let h = IoHandle::Stdin {
            buffer: vec![1, 2, 3],
        };
        assert_eq!(h.strategy(), IoStrategy::Cached);
        assert_eq!(h.kind_name(), "Stdin");
    }

    #[test]
    fn stdout_is_always_cached() {
        let h = IoHandle::Stdout { buffer: vec![] };
        assert_eq!(h.strategy(), IoStrategy::Cached);
        assert_eq!(h.kind_name(), "Stdout");
    }

    #[test]
    fn stderr_is_always_cached() {
        let h = IoHandle::Stderr { buffer: vec![] };
        assert_eq!(h.strategy(), IoStrategy::Cached);
        assert_eq!(h.kind_name(), "Stderr");
    }

    #[test]
    fn http_handle_strategy() {
        let h = IoHandle::HttpConnection {
            url: "https://api.example.com/data".into(),
            method: HttpMethod::Get,
            body: None,
            last_response: Some(b"{\"ok\":true}".to_vec()),
            strategy: IoStrategy::Replay,
                    };
        assert_eq!(h.strategy(), IoStrategy::Replay);
        assert_eq!(h.kind_name(), "HttpConnection");
    }

    #[test]
    fn timer_handle_strategy() {
        let h = IoHandle::Timer {
            ms: 5000,
            strategy: IoStrategy::Replay,
                    };
        assert_eq!(h.strategy(), IoStrategy::Replay);
        assert_eq!(h.kind_name(), "Timer");
    }

    #[test]
    fn tcp_handle_strategy() {
        let h = IoHandle::TcpStream {
            addr: "127.0.0.1:8080".into(),
            strategy: IoStrategy::Replay,
                stream: None,
                last_request: None,
                last_response: None
        };
        assert_eq!(h.strategy(), IoStrategy::Replay);
        assert_eq!(h.kind_name(), "TcpStream");
    }

    #[test]
    fn all_kind_names() {
        let kinds: &[(&str, IoHandle)] = &[
            (
                "File",
                IoHandle::File {
                    path: "a".into(),
                    mode: FileMode::Read,
                    position: 0,
                    strategy: IoStrategy::Seek,
                file: None,
                cached: None,
                },
            ),
            (
                "TcpStream",
                IoHandle::TcpStream {
                    addr: "x".into(),
                    strategy: IoStrategy::Replay,
                stream: None,
                last_request: None,
                last_response: None
                },
            ),
            (
                "HttpConnection",
                IoHandle::HttpConnection {
                    url: "x".into(),
                    method: HttpMethod::Get,
                    body: None,
                    last_response: None,
                    strategy: IoStrategy::Replay,
                                    },
            ),
            (
                "Timer",
                IoHandle::Timer {
                    ms: 0,
                    strategy: IoStrategy::Replay,
                                    },
            ),
            ("Stdin", IoHandle::Stdin { buffer: vec![] }),
            ("Stdout", IoHandle::Stdout { buffer: vec![] }),
            ("Stderr", IoHandle::Stderr { buffer: vec![] }),
        ];
        for (expected, handle) in kinds {
            assert_eq!(handle.kind_name(), *expected);
        }
    }
    #[test]
    fn clone_file_sets_file_to_none() {
        let h = IoHandle::File {
            path: "/tmp/clone_test.txt".into(),
            mode: FileMode::Read,
            position: 42,
            strategy: IoStrategy::Seek,
            file: None,
            cached: Some(b"cached data".to_vec()),
        };
        let cloned = h.clone();
        if let IoHandle::File { file, cached, position, .. } = cloned {
            assert!(file.is_none(), "cloned File should have file=None");
            assert_eq!(position, 42);
            assert_eq!(cached, Some(b"cached data".to_vec()));
        } else {
            panic!("expected File");
        }
    }

    #[test]
    fn clone_tcp_sets_stream_to_none() {
        let h = IoHandle::TcpStream {
            addr: "127.0.0.1:8080".into(),
            strategy: IoStrategy::Replay,
            stream: None,
            last_request: Some(b"req".to_vec()),
            last_response: Some(b"resp".to_vec()),
        };
        let cloned = h.clone();
        if let IoHandle::TcpStream { stream, last_request, last_response, .. } = cloned {
            assert!(stream.is_none(), "cloned TcpStream should have stream=None");
            assert_eq!(last_request, Some(b"req".to_vec()));
            assert_eq!(last_response, Some(b"resp".to_vec()));
        } else {
            panic!("expected TcpStream");
        }
    }

    #[test]
    fn clone_http_preserves_all_fields() {
        let h = IoHandle::HttpConnection {
            url: "https://example.com".into(),
            method: HttpMethod::Post,
            body: Some(b"payload".to_vec()),
            last_response: Some(b"200 OK".to_vec()),
            strategy: IoStrategy::Replay,
        };
        let cloned = h.clone();
        if let IoHandle::HttpConnection { url, method, body, last_response, strategy } = cloned {
            assert_eq!(url, "https://example.com");
            assert_eq!(method, HttpMethod::Post);
            assert_eq!(body, Some(b"payload".to_vec()));
            assert_eq!(last_response, Some(b"200 OK".to_vec()));
            assert_eq!(strategy, IoStrategy::Replay);
        } else {
            panic!("expected HttpConnection");
        }
    }

    #[test]
    fn tcp_echo_roundtrip() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("TcpListener::bind failed -- network unavailable?");
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 128];
            let n = stream.read(&mut buf).unwrap();
            stream.write_all(&buf[..n]).unwrap();
        });

        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        stream.write_all(b"pausible").unwrap();
        let mut buf = [0u8; 128];
        let n = stream.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"pausible");

        handle.join().unwrap();
    }
}
