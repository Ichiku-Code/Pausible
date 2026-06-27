use core::fmt;

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
#[derive(Debug, Clone)]
pub enum IoHandle {
    File {
        path: String,
        mode: FileMode,
        position: u64,
        strategy: IoStrategy,
    },
    TcpStream {
        addr: String,
        strategy: IoStrategy,
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
        };
        assert_eq!(h.strategy(), IoStrategy::Seek);
        assert_eq!(h.kind_name(), "File");
    }

    #[test]
    fn stdin_is_always_cached() {
        let h = IoHandle::Stdin { buffer: vec![1, 2, 3] };
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
        };
        assert_eq!(h.strategy(), IoStrategy::Replay);
        assert_eq!(h.kind_name(), "TcpStream");
    }

    #[test]
    fn all_kind_names() {
        let kinds: &[(&str, IoHandle)] = &[
            ("File", IoHandle::File { path: "a".into(), mode: FileMode::Read, position: 0, strategy: IoStrategy::Seek }),
            ("TcpStream", IoHandle::TcpStream { addr: "x".into(), strategy: IoStrategy::Replay }),
            ("HttpConnection", IoHandle::HttpConnection { url: "x".into(), method: HttpMethod::Get, body: None, last_response: None, strategy: IoStrategy::Replay }),
            ("Timer", IoHandle::Timer { ms: 0, strategy: IoStrategy::Replay }),
            ("Stdin", IoHandle::Stdin { buffer: vec![] }),
            ("Stdout", IoHandle::Stdout { buffer: vec![] }),
            ("Stderr", IoHandle::Stderr { buffer: vec![] }),
        ];
        for (expected, handle) in kinds {
            assert_eq!(handle.kind_name(), *expected);
        }
    }
}
