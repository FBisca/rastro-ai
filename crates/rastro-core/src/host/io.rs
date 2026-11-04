//! Talking to a connector without trusting it: every queue bounded, every wait timed, and
//! every helper thread counted so teardown can be shown to release them.
use crate::display_safe;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{ChildStderr, ChildStdin},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, SyncSender, channel, sync_channel},
    },
    thread,
    time::Duration,
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

/// The one place the core prints connector-related lines: a single thread per process with a
/// bounded queue, shared by every run. A terminal that isn't drained stalls that thread and
/// fills the queue; further lines are dropped, and nothing else ever waits on it.
#[derive(Clone)]
pub struct Printer {
    queue: SyncSender<Message>,
    queued: Arc<AtomicUsize>,
    stalled: Arc<AtomicBool>,
}
/// Printer threads started, for tests to show runs don't add any.
#[cfg(test)]
static PRINTER_THREADS: AtomicUsize = AtomicUsize::new(0);

enum Message {
    Line(String),
    Flush(SyncSender<()>),
}
impl Printer {
    /// The process-wide printer, started on first use, writing to stderr.
    pub fn global() -> &'static Printer {
        static PRINTER: OnceLock<Printer> = OnceLock::new();
        PRINTER.get_or_init(|| Printer::start(Box::new(std::io::stderr())))
    }
    #[cfg(test)]
    pub fn with_sink(sink: Box<dyn Write + Send>) -> Printer {
        PRINTER_THREADS.fetch_add(1, Ordering::SeqCst);
        Printer::start(sink)
    }
    fn start(mut sink: Box<dyn Write + Send>) -> Printer {
        let (queue, messages) = sync_channel::<Message>(RELAY_QUEUE);
        let printer = Printer {
            queue,
            queued: Arc::new(AtomicUsize::new(0)),
            stalled: Arc::new(AtomicBool::new(false)),
        };
        let (queued, stalled) = (Arc::clone(&printer.queued), Arc::clone(&printer.stalled));
        thread::spawn(move || {
            for message in messages {
                match message {
                    Message::Line(line) => {
                        queued.fetch_sub(1, Ordering::SeqCst);
                        let _ = writeln!(sink, "{line}");
                        stalled.store(false, Ordering::SeqCst);
                    }
                    Message::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        printer
    }
    fn push(&self, line: String) -> bool {
        let sent = self.queue.try_send(Message::Line(line)).is_ok();
        if sent {
            self.queued.fetch_add(1, Ordering::SeqCst);
        }
        sent
    }
    /// Waits, for at most `timeout`, until what is queued has been printed. If the printer is
    /// known to be stuck it doesn't wait at all, so a blocked terminal costs one timeout, not
    /// one per run.
    pub fn flush(&self, timeout: Duration) {
        if self.stalled.load(Ordering::SeqCst) {
            return;
        }
        let (done, printed) = sync_channel(1);
        if self.queue.try_send(Message::Flush(done)).is_err() {
            return;
        }
        if printed.recv_timeout(timeout).is_err() {
            self.stalled.store(true, Ordering::SeqCst);
        }
    }
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::SeqCst)
    }
}

/// One run's view of the printer: at most `max` lines are printed per run, each escaped and
/// tagged with the plugin; the rest are dropped and counted, and a notice says how many.
#[derive(Clone)]
pub struct Relay(Arc<RelayState>);
struct RelayState {
    printer: Printer,
    plugin: String,
    dropped: AtomicUsize,
    used: AtomicUsize,
    max: usize,
}
impl Relay {
    pub fn line(&self, text: &str) {
        let state = &self.0;
        let fits = state.used.fetch_add(1, Ordering::Relaxed) < state.max;
        let line = format!("[{}] {}", state.plugin, display_safe(text));
        if !fits || !state.printer.push(line) {
            state.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
impl Drop for RelayState {
    fn drop(&mut self) {
        let count = self.dropped.load(Ordering::Relaxed);
        if count > 0 {
            self.printer
                .push(format!("[{}] dropped {count} stderr lines", self.plugin));
        }
    }
}

pub fn start_relay(printer: &Printer, plugin: String, max: usize) -> Relay {
    Relay(Arc::new(RelayState {
        printer: printer.clone(),
        plugin,
        dropped: AtomicUsize::new(0),
        used: AtomicUsize::new(0),
        max,
    }))
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
    use std::time::Instant;

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

    /// A sink that blocks until its sender is dropped, like a terminal nobody reads.
    struct Blocked(Receiver<()>);
    impl Write for Blocked {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn S9m_one_printer_and_a_bounded_queue_serve_many_runs_on_a_blocked_sink() {
        let (release, blocked) = channel();
        let printer = Printer::with_sink(Box::new(Blocked(blocked)));
        let before = PRINTER_THREADS.load(Ordering::SeqCst);
        for run in 0..20 {
            let relay = start_relay(&printer, format!("run{run}"), 1000);
            for _ in 0..1000 {
                relay.line("noise");
            }
        }
        assert_eq!(PRINTER_THREADS.load(Ordering::SeqCst), before);
        assert!(
            printer.queued() <= RELAY_QUEUE,
            "{} lines queued",
            printer.queued()
        );
        // A stuck printer is waited on once, not once per run.
        let started = Instant::now();
        for _ in 0..20 {
            printer.flush(Duration::from_millis(100));
        }
        assert!(started.elapsed() < Duration::from_millis(1000));
        drop(release);
    }
}
