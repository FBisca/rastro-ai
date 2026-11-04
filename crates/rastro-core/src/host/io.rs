//! Talking to a connector without trusting it: every queue bounded, every wait timed, every
//! helper thread able to be abandoned.
use crate::display_safe;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, SyncSender, channel, sync_channel},
    },
    thread,
};

/// Lines held between the reader thread and the host. When it is full the reader stops
/// reading, the pipe fills, and the connector waits.
pub const INBOX_LINES: usize = 64;
/// Stderr lines queued for printing; more are dropped and counted.
const RELAY_QUEUE: usize = 256;

pub enum Event {
    Line(String),
    /// A line longer than the limit; the rest is never read.
    TooLong,
    /// A line that isn't UTF-8.
    Invalid,
    Eof,
}

/// Reads stdout lines on a thread so the host can wait with a timeout. The thread ends at
/// end of input, which killing the child brings, or when the host drops the receiver.
pub fn read_lines(stdout: ChildStdout, max_line: usize) -> Receiver<Event> {
    let (sender, receiver) = sync_channel(INBOX_LINES);
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
                Ok(_) => match String::from_utf8(buffer) {
                    Ok(line) => Event::Line(line),
                    Err(_) => Event::Invalid,
                },
            };
            let last = !matches!(event, Event::Line(_));
            if sender.send(event).is_err() || last {
                break;
            }
        }
    });
    receiver
}

/// A write job and where to report how it went.
pub type Job = (String, SyncSender<std::io::Result<()>>);

/// Writes to the connector's stdin on its own thread, so a connector that stops reading
/// can't block the host: the host waits for the acknowledgement with a deadline instead.
/// Dropping the sender ends the thread and closes stdin.
pub fn write_lines(mut stdin: ChildStdin) -> Sender<Job> {
    let (sender, receiver) = channel::<Job>();
    thread::spawn(move || {
        for (line, ack) in receiver {
            let result = writeln!(stdin, "{line}").and_then(|()| stdin.flush());
            let _ = ack.send(result);
        }
    });
    sender
}

/// Drains stderr so the connector never blocks on it, and prints at most `max_lines` lines,
/// each escaped. The rest are dropped and counted: a notice says how many. Printing happens
/// on its own thread, so a blocked terminal stalls neither the drain nor the host. The
/// returned receiver gets a message once everything is printed.
pub fn relay_stderr(stderr: ChildStderr, plugin: String, max_lines: usize) -> Receiver<()> {
    let (queue, lines) = sync_channel::<String>(RELAY_QUEUE);
    let (done, finished) = channel();
    let dropped = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&dropped);
    let name = plugin.clone();
    thread::spawn(move || {
        let mut terminal = std::io::stderr();
        for line in lines {
            let _ = writeln!(terminal, "[{name}] {line}");
        }
        let count = counted.swap(0, Ordering::Relaxed);
        if count > 0 {
            let _ = writeln!(terminal, "[{name}] dropped {count} stderr lines");
        }
        let _ = done.send(());
    });
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut relayed = 0;
        loop {
            let mut buffer = Vec::new();
            // Bounded reads: a long line is relayed in pieces, never held whole.
            match reader.by_ref().take(4096).read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) if relayed < max_lines => {
                    relayed += 1;
                    let text = String::from_utf8_lossy(&buffer);
                    let line = display_safe(text.trim_end_matches(['\n', '\r']));
                    if queue.try_send(line).is_err() {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(_) => {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });
    finished
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        time::Duration,
    };

    #[test]
    fn S9b_reader_holds_a_bounded_number_of_lines_and_the_writer_waits() {
        let mut child = Command::new("sh")
            .args(["-c", "yes | head -n 2000000"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let inbox = read_lines(child.stdout.take().unwrap(), 1024);
        thread::sleep(Duration::from_millis(400));
        // Unread, the producer is stuck on a full pipe: 4 MB of output didn't fit anywhere.
        assert!(
            child.try_wait().unwrap().is_none(),
            "output was buffered without bound"
        );
        drop(inbox);
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
