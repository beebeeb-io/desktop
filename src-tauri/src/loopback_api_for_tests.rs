    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// How the API answers each request.
    #[derive(Clone, Copy)]
    enum Answer {
        With { status: &'static str, body: &'static str },
        /// Reads the request and never answers: the connection stays open until the API is dropped.
        Never,
    }

    /// A loopback API that records each raw request it reads (before it answers, so a request is recorded by the time
    /// its client has the answer) and answers it as told. Dropping it stops its thread and closes every connection.
    pub(super) struct LoopbackApi {
        pub(super) base: String,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl LoopbackApi {
        /// Answers every request with `status` and `body`.
        pub(super) fn start(status: &'static str, body: &'static str) -> Self {
            Self::answering(Answer::With { status, body })
        }

        /// Never answers a request.
        pub(super) fn never_answering() -> Self {
            Self::answering(Answer::Never)
        }

        fn answering(answer: Answer) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (seen, halt) = (requests.clone(), stop.clone());
            let handle = std::thread::spawn(move || {
                // The connections of `Answer::Never`, open until this thread ends.
                let mut held = Vec::new();
                while !halt.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                            // Up to the end of the headers (or what one read gave, if the peer stops there).
                            let mut request = Vec::new();
                            let mut buf = [0u8; 8192];
                            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                                match stream.read(&mut buf) {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => request.extend_from_slice(&buf[..n]),
                                }
                            }
                            seen.lock()
                                .unwrap()
                                .push(String::from_utf8_lossy(&request).into_owned());
                            match answer {
                                Answer::With { status, body } => {
                                    let _ = write!(
                                        stream,
                                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                                        body.len()
                                    );
                                }
                                Answer::Never => held.push(stream),
                            }
                        }
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                    }
                }
            });
            Self {
                base,
                requests,
                stop,
                handle: Some(handle),
            }
        }

        pub(super) fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for LoopbackApi {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }
