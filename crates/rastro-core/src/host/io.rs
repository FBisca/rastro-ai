//! Talking to a connector without trusting it: every queue bounded, every wait timed, and
//! every helper thread counted so teardown can be shown to release them.
use crate::display_safe;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{ChildStderr, ChildStdin},
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
/// Lines queued for printing; more are dropped and counted.
const RELAY_QUEUE: usize = 256;

/// Counts the helper threads of one run that are still alive.
#[derive(Clone, Default)]
pub struct Tally(Arc<AtomicUsize>);
impl Tally {
    #[cfg(test)]
    pub fn live(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
    fn guard(&self) -> Guard {
        self.0.fetch_add(1, Ordering::SeqCst);
        Guard(Arc::clone(&self.0))
    }
}
struct Guard(Arc<AtomicUsize>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub enum Event {
    Line(String),
    /// A line longer than the limit; the rest is never read.
    TooLong,
    /// A line that isn't UTF-8.
    Invalid,
    Eof,
}

/// Reads lines on a thread so the host can wait with a timeout. The thread ends at end of
/// input, which killing the connector's process group brings, or when the host drops the
/// receiver.
pub fn read_lines<R: Read + Send + 'static>(
    stdout: R,
    max_line: usize,
    tally: &Tally,
) -> Receiver<Event> {
    let (sender, receiver) = sync_channel(INBOX_LINES);
    let guard = tally.guard();
    thread::spawn(move || {
        let _guard = guard;
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
pub fn write_lines(mut stdin: ChildStdin, tally: &Tally) -> Sender<Job> {
    let (sender, receiver) = channel::<Job>();
    let guard = tally.guard();
    thread::spawn(move || {
        let _guard = guard;
        for (line, ack) in receiver {
            let result = writeln!(stdin, "{line}").and_then(|()| stdin.flush());
            let _ = ack.send(result);
        }
    });
    sender
}

/// Where everything the core says about a connector goes: its stderr and the core's own
/// diagnostics about it. At most `max` lines are printed per run, each escaped; the rest are
/// dropped and counted, and a notice says how many. Printing happens on its own thread, so a
/// terminal that isn't drained stalls neither the host nor the connector.
#[derive(Clone)]
pub struct Relay {
    queue: SyncSender<String>,
    dropped: Arc<AtomicUsize>,
    used: Arc<AtomicUsize>,
    max: usize,
}
impl Relay {
    pub fn line(&self, text: &str) {
        let fits = self.used.fetch_add(1, Ordering::Relaxed) < self.max;
        if !fits || self.queue.try_send(display_safe(text)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Starts the printer. The receiver gets a message once every `Relay` clone is dropped and
/// everything queued is printed.
pub fn start_relay(plugin: String, max: usize, tally: &Tally) -> (Relay, Receiver<()>) {
    let (queue, lines) = sync_channel::<String>(RELAY_QUEUE);
    let (done, finished) = channel();
    let dropped = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&dropped);
    let guard = tally.guard();
    thread::spawn(move || {
        let _guard = guard;
        let mut terminal = std::io::stderr();
        for line in lines {
            let _ = writeln!(terminal, "[{plugin}] {line}");
        }
        let count = counted.swap(0, Ordering::Relaxed);
        if count > 0 {
            let _ = writeln!(terminal, "[{plugin}] dropped {count} stderr lines");
        }
        let _ = done.send(());
    });
    let relay = Relay {
        queue,
        dropped,
        used: Arc::new(AtomicUsize::new(0)),
        max,
    };
    (relay, finished)
}

/// Drains the connector's stderr into the relay, in bounded pieces, so it never blocks.
pub fn drain_stderr(stderr: ChildStderr, relay: Relay, tally: &Tally) {
    let guard = tally.guard();
    thread::spawn(move || {
        let _guard = guard;
        let mut reader = BufReader::new(stderr);
        loop {
            let mut buffer = Vec::new();
            match reader.by_ref().take(4096).read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    relay.line(String::from_utf8_lossy(&buffer).trim_end_matches(['\n', '\r']))
                }
            }
        }
    });
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Endless `y\n` lines, counting the bytes handed out.
    struct Endless(Arc<AtomicUsize>);
    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            for pair in buf.chunks_mut(2) {
                pair.copy_from_slice(&b"y\n"[..pair.len()]);
            }
            self.0.fetch_add(buf.len(), Ordering::SeqCst);
            Ok(buf.len())
        }
    }

    #[test]
    fn S9b_reader_stops_reading_when_the_inbox_is_full() {
        let consumed = Arc::new(AtomicUsize::new(0));
        let tally = Tally::default();
        let inbox = read_lines(Endless(Arc::clone(&consumed)), 1024, &tally);
        // Nobody receives. The reader must stall; wait until its progress stops.
        let mut last = 0;
        let mut still = 0;
        let started = Instant::now();
        while still < 5 {
            thread::sleep(Duration::from_millis(20));
            let now = consumed.load(Ordering::SeqCst);
            assert!(now < 1 << 20, "reader kept consuming: {now} bytes");
            assert!(started.elapsed() < Duration::from_secs(10));
            still = if now == last { still + 1 } else { 0 };
            last = now;
        }
        // What it holds: the inbox, one event in hand and one buffer fill.
        assert!(
            last <= 2 * INBOX_LINES + 2 * 8192 + 2 * 8192,
            "{last} bytes read"
        );
        drop(inbox);
        let started = Instant::now();
        while tally.live() > 0 {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "reader did not end"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
