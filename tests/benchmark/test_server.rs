use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Copy)]
pub enum Behavior {
    // Answer with the status after the delay
    Respond { status: u16, delay: Duration },
    // Accept the request and never answer
    Hang,
}

impl Behavior {
    pub fn status(status: u16) -> Self {
        Behavior::Respond {
            status,
            delay: Duration::ZERO,
        }
    }

    pub fn delayed(status: u16, delay: Duration) -> Self {
        Behavior::Respond { status, delay }
    }
}

#[derive(Default)]
struct Shared {
    total: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    heads: Mutex<Vec<String>>,
    stop: AtomicBool,
}

// Minimal keep-alive HTTP server that records what it receives
pub struct TestServer {
    pub port: u16,
    shared: Arc<Shared>,
    accept_thread: Option<JoinHandle<()>>,
}

impl TestServer {
    pub fn start(behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();

        listener.set_nonblocking(true).unwrap();

        let port = listener.local_addr().unwrap().port();
        let shared = Arc::new(Shared::default());
        let accept_shared = Arc::clone(&shared);

        let accept_thread = thread::spawn(move || accept_loop(listener, accept_shared, behavior));

        Self {
            port,
            shared,
            accept_thread: Some(accept_thread),
        }
    }

    pub fn total_requests(&self) -> usize {
        self.shared.total.load(Ordering::SeqCst)
    }

    pub fn max_in_flight(&self) -> usize {
        self.shared.max_in_flight.load(Ordering::SeqCst)
    }

    // Raw request heads, lowercased for case-insensitive header checks
    pub fn heads(&self) -> Vec<String> {
        self.shared
            .heads
            .lock()
            .unwrap()
            .iter()
            .map(|head| head.to_lowercase())
            .collect()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);

        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, behavior: Behavior) {
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let connection_shared = Arc::clone(&shared);

                thread::spawn(move || serve_connection(stream, connection_shared, behavior));
            }
            Err(_) => thread::sleep(Duration::from_millis(1)),
        }
    }
}

fn serve_connection(stream: TcpStream, shared: Arc<Shared>, behavior: Behavior) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

    let Ok(mut writer) = stream.try_clone() else {
        return;
    };

    let mut reader = BufReader::new(stream);

    while let Some(head) = read_head(&mut reader) {
        shared.total.fetch_add(1, Ordering::SeqCst);
        shared.heads.lock().unwrap().push(head);

        let current = shared.in_flight.fetch_add(1, Ordering::SeqCst) + 1;

        shared.max_in_flight.fetch_max(current, Ordering::SeqCst);

        let keep_going = respond(&mut writer, &shared, behavior);

        shared.in_flight.fetch_sub(1, Ordering::SeqCst);

        if !keep_going {
            break;
        }
    }
}

fn read_head(reader: &mut BufReader<TcpStream>) -> Option<String> {
    let mut head = String::new();

    loop {
        let mut line = String::new();

        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }

        head.push_str(&line);

        if line == "\r\n" {
            return Some(head);
        }
    }
}

fn respond(writer: &mut TcpStream, shared: &Shared, behavior: Behavior) -> bool {
    match behavior {
        Behavior::Respond { status, delay } => {
            thread::sleep(delay);

            let response = format!("HTTP/1.1 {status} Test\r\nContent-Length: 2\r\n\r\nok");

            writer.write_all(response.as_bytes()).is_ok()
        }
        Behavior::Hang => {
            while !shared.stop.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(20));
            }

            false
        }
    }
}
