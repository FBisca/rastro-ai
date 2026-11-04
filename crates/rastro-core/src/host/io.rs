//! Reading a connector's output without trusting it: bounded lines, on their own threads.
use crate::display_safe;
use std::{
    io::{BufRead, BufReader, Read},
    process::{ChildStderr, ChildStdout},
    sync::mpsc::{Receiver, channel},
    thread::{self, JoinHandle},
};

pub enum Event {
    Line(String),
    /// A line longer than the limit; the rest is never read.
    TooLong,
    Eof,
}

/// Reads stdout lines on a thread so the host can wait with a timeout. The thread ends at
/// end of input, which killing the child brings.
pub fn read_lines(stdout: ChildStdout, max_line: usize) -> Receiver<Event> {
    let (sender, receiver) = channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut buffer = Vec::new();
            let read = reader
                .by_ref()
                .take(max_line as u64 + 1)
                .read_until(b'\n', &mut buffer);
            let event = match read {
                Ok(0) | Err(_) => Event::Eof,
                Ok(_) if buffer.last() != Some(&b'\n') && buffer.len() > max_line => Event::TooLong,
                Ok(_) => Event::Line(String::from_utf8_lossy(&buffer).into_owned()),
            };
            let last = !matches!(event, Event::Line(_));
            if sender.send(event).is_err() || last {
                break;
            }
        }
    });
    receiver
}

/// Drains stderr so the connector never blocks on it, and relays each line safely.
pub fn relay_stderr(stderr: ChildStderr, plugin: String) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        loop {
            let mut buffer = Vec::new();
            // Bounded reads: a long line is relayed in pieces, never held whole.
            match reader.by_ref().take(4096).read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buffer);
                    eprintln!(
                        "[{plugin}] {}",
                        display_safe(text.trim_end_matches(['\n', '\r']))
                    );
                }
            }
        }
    })
}
