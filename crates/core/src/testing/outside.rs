//! The world outside, as a test plays it: an HTTP server of the test's own,
//! and one that hangs up.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// The world outside, as the test plays it: an HTTP server on this machine
// that answers `/slow` after a pause, `/echo` with the body it was sent, and
// any other path with its own name, and keeps what it was asked.
pub struct Outside {
  address: String,
  asked: Arc<Mutex<Vec<String>>>,
}

// How long `/slow` pauses before it answers.
pub const SLOW: Duration = Duration::from_millis(300);

impl Outside {
  pub fn start() -> Outside {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let asked = Arc::new(Mutex::new(vec![]));
    let keeping = Arc::clone(&asked);
    std::thread::spawn(move || {
      for stream in listener.incoming().flatten() {
        let keeping = Arc::clone(&keeping);
        std::thread::spawn(move || answer(stream, &keeping));
      }
    });
    Outside { address, asked }
  }

  pub fn url(&self, path: &str) -> String {
    format!("http://{}{path}", self.address)
  }

  // What it was asked so far: each request's method and path.
  pub fn asked(&self) -> Vec<String> {
    self.asked.lock().unwrap().clone()
  }
}

// The address of a server that hangs up on every connection, before saying anything.
pub fn hanging_up() -> String {
  let listener = TcpListener::bind("127.0.0.1:0").unwrap();
  let address = listener.local_addr().unwrap().to_string();
  std::thread::spawn(move || listener.incoming().for_each(drop));
  address
}

// Read one request and answer it.
fn answer(stream: TcpStream, asked: &Mutex<Vec<String>>) {
  let mut reader = BufReader::new(stream);
  let mut line = String::new();
  reader.read_line(&mut line).unwrap();
  let [method, path] = [0, 1].map(|at| line.split_whitespace().nth(at).unwrap_or_default().to_string());
  let mut length = 0;
  loop {
    let mut header = String::new();
    reader.read_line(&mut header).unwrap();
    if header.trim().is_empty() {
      break;
    }
    if let Some((name, value)) = header.split_once(':') && name.eq_ignore_ascii_case("content-length") {
      length = value.trim().parse().unwrap();
    }
  }
  let mut body = vec![0; length];
  reader.read_exact(&mut body).unwrap();
  asked.lock().unwrap().push(format!("{method} {path}"));
  let answered = match path.as_str() {
    "/slow" => {
      std::thread::sleep(SLOW);
      b"slow".to_vec()
    }
    "/echo" => body,
    other => other.trim_start_matches('/').as_bytes().to_vec(),
  };
  let mut stream = reader.into_inner();
  let head = format!("HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", answered.len());
  stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(&answered)).unwrap();
}
