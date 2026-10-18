//! Task queues: a priority queue drained by the `android_main` thread (GPUI's
//! main thread, woken through the activity's looper), worker threads for
//! background tasks and one timer thread. Same model as `gpui_linux`.

use android_activity::AndroidAppWaker;
use gpui::{PlatformDispatcher, Priority, PriorityQueueReceiver, PriorityQueueSender, RunnableVariant};
use std::collections::BinaryHeap;
use std::sync::mpsc;
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

const MIN_THREADS: usize = 2;

struct Timer {
    at: Instant,
    seq: u64,
    runnable: RunnableVariant,
}

impl PartialEq for Timer {
    fn eq(&self, other: &Self) -> bool {
        self.at == other.at && self.seq == other.seq
    }
}
impl Eq for Timer {}
impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Timer {
    // Min-heap on (at, seq).
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (other.at, other.seq).cmp(&(self.at, self.seq))
    }
}

pub(crate) struct AndroidDispatcher {
    main_sender: PriorityQueueSender<RunnableVariant>,
    waker: AndroidAppWaker,
    background_sender: PriorityQueueSender<RunnableVariant>,
    timer_sender: mpsc::Sender<(Duration, RunnableVariant)>,
    main_thread_id: ThreadId,
}

impl AndroidDispatcher {
    /// Must be created on the `android_main` thread. Returns the receiver the
    /// run loop drains.
    pub fn new(waker: AndroidAppWaker) -> (Self, PriorityQueueReceiver<RunnableVariant>) {
        let (main_sender, main_receiver) = PriorityQueueReceiver::new();
        let (background_sender, background_receiver) = PriorityQueueReceiver::<RunnableVariant>::new();
        let threads = thread::available_parallelism().map_or(MIN_THREADS, |n| n.get().max(MIN_THREADS));
        for i in 0..threads {
            let receiver = background_receiver.clone();
            thread::Builder::new()
                .name(format!("Worker-{i}"))
                .spawn(move || {
                    for runnable in receiver.iter() {
                        runnable.run();
                    }
                })
                .expect("spawn worker thread");
        }

        let (timer_sender, timer_receiver) = mpsc::channel::<(Duration, RunnableVariant)>();
        thread::Builder::new()
            .name("Timer".into())
            .spawn(move || {
                let mut heap = BinaryHeap::<Timer>::new();
                let mut seq = 0u64;
                loop {
                    let next = heap.peek().map(|t| t.at);
                    let received = match next {
                        Some(at) => timer_receiver.recv_timeout(at.saturating_duration_since(Instant::now())),
                        None => timer_receiver.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                    };
                    match received {
                        Ok((after, runnable)) => {
                            seq += 1;
                            heap.push(Timer { at: Instant::now() + after, seq, runnable });
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        // The dispatcher is gone; pending timers could only run
                        // tasks nothing observes any more.
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                    let now = Instant::now();
                    while heap.peek().is_some_and(|t| t.at <= now) {
                        if let Some(timer) = heap.pop() {
                            timer.runnable.run();
                        }
                    }
                }
            })
            .expect("spawn timer thread");

        let dispatcher = Self {
            main_sender,
            waker,
            background_sender,
            timer_sender,
            main_thread_id: thread::current().id(),
        };
        (dispatcher, main_receiver)
    }
}

impl PlatformDispatcher for AndroidDispatcher {
    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread_id
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        self.background_sender
            .send(priority, runnable)
            .unwrap_or_else(|_| panic!("background queue closed"));
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        match self.main_sender.send(priority, runnable) {
            Ok(()) => self.waker.wake(),
            // The run loop is gone (shutdown). The runnable may hold a !Send
            // future, so it must not be dropped on this thread.
            Err(e) => std::mem::forget(e),
        }
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        if let Err(e) = self.timer_sender.send((duration, runnable)) {
            // Timer thread gone (shutdown): leave the task pending.
            std::mem::forget(e);
        }
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        thread::spawn(f);
    }
}
