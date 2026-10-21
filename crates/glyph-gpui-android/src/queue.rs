//! Task queue with GPUI's priorities. GPUI exports its own
//! `PriorityQueueReceiver` only for Windows, Linux and the web, so Android
//! brings this small one: high before medium before low, and every eighth pop
//! takes the oldest lower-priority task so nothing starves.

use gpui::Priority;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

struct Queues<T> {
    high: VecDeque<T>,
    medium: VecDeque<T>,
    low: VecDeque<T>,
    pops: u32,
}

impl<T> Queues<T> {
    fn pop(&mut self) -> Option<T> {
        self.pops = self.pops.wrapping_add(1);
        if self.pops % 8 == 0 {
            if let Some(t) = self.low.pop_front().or_else(|| self.medium.pop_front()) {
                return Some(t);
            }
        }
        self.high.pop_front().or_else(|| self.medium.pop_front()).or_else(|| self.low.pop_front())
    }
}

struct Shared<T> {
    queues: Mutex<Queues<T>>,
    ready: Condvar,
}

pub(crate) struct Sender<T>(Arc<Shared<T>>);
pub(crate) struct Receiver<T>(Arc<Shared<T>>);

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub(crate) fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        queues: Mutex::new(Queues { high: VecDeque::new(), medium: VecDeque::new(), low: VecDeque::new(), pops: 0 }),
        ready: Condvar::new(),
    });
    (Sender(shared.clone()), Receiver(shared))
}

impl<T> Sender<T> {
    pub fn send(&self, priority: Priority, item: T) {
        let mut q = self.0.queues.lock().unwrap();
        match priority {
            Priority::RealtimeAudio | Priority::High => q.high.push_back(item),
            Priority::Medium => q.medium.push_back(item),
            Priority::Low => q.low.push_back(item),
        }
        drop(q);
        self.0.ready.notify_one();
    }
}

impl<T> Receiver<T> {
    pub fn try_pop(&self) -> Option<T> {
        self.0.queues.lock().unwrap().pop()
    }

    /// Blocks until a task is queued.
    pub fn pop(&self) -> T {
        let mut q = self.0.queues.lock().unwrap();
        loop {
            if let Some(t) = q.pop() {
                return t;
            }
            q = self.0.ready.wait(q).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_first_but_low_is_not_starved() {
        let (tx, rx) = channel();
        for i in 0..10 {
            tx.send(Priority::High, i);
        }
        tx.send(Priority::Low, 100);
        let order: Vec<i32> = std::iter::from_fn(|| rx.try_pop()).collect();
        assert_eq!(order[0], 0);
        assert_eq!(order[7], 100);
        assert_eq!(order.len(), 11);
    }
}
