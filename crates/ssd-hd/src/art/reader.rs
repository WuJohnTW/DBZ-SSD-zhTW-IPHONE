//! A thread that reads art ahead of the frames that draw it: the frames ask for what they want, in
//! order, and take what the thread has read since. The field maps' tiles ([`super::field`]) and the
//! pack's other art ([`super::load`]) each have one.

use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::hash::Hash;
use std::sync::{Arc, Condvar, Mutex};

/// What the drawing and the reading threads share: the jobs to read, in order, the jobs read and not
/// taken yet, and whether the reader is gone.
struct Shared<K, J, R> {
    jobs: VecDeque<(K, J)>,
    read: Vec<(K, J, R)>,
    closed: bool,
}

/// [`Shared`] and the condition the reading thread waits on for a job.
type Channel<K, J, R> = (Mutex<Shared<K, J, R>>, Condvar);

/// A thread that reads the jobs it is asked for (`J`, known by their key `K`) into `R`; it ends when
/// this is dropped.
pub(crate) struct Reader<K, J, R> {
    shared: Arc<Channel<K, J, R>>,
    /// The jobs asked for that have not come back.
    pub(crate) asked: FxHashSet<K>,
    /// The thread: once it has ended while the reader is there, it panicked.
    worker: std::thread::JoinHandle<()>,
}

impl<K, J, R> Reader<K, J, R>
where
    K: Copy + Eq + Hash + Send + 'static,
    J: Send + 'static,
    R: Send + 'static,
{
    /// The reader, its thread (named `name`) started, that reads a job with `read`; `None` when the
    /// thread cannot be.
    pub(crate) fn start(name: &str, read: impl Fn(&J) -> R + Send + 'static) -> Option<Self> {
        let shared = Arc::new((
            Mutex::new(Shared { jobs: VecDeque::new(), read: Vec::new(), closed: false }),
            Condvar::new(),
        ));
        let theirs = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                while let Some((key, job)) = next_job(&theirs) {
                    let art = read(&job);
                    let Ok(mut shared) = theirs.0.lock() else { return };
                    shared.read.push((key, job, art));
                }
            })
            .ok()?;
        Some(Self { shared, asked: FxHashSet::default(), worker })
    }

    /// Whether the thread still runs (it only ends on its own by a panic).
    pub(crate) fn alive(&self) -> bool {
        !self.worker.is_finished()
    }

    /// Asks for `jobs`, in order, in place of the jobs asked for before that the thread has not
    /// started reading; a job whose key is being read or read and not taken is not asked again.
    pub(crate) fn ask(&mut self, jobs: Vec<(K, J)>) {
        let (lock, wake) = &*self.shared;
        let Ok(mut shared) = lock.lock() else { return };
        for (key, _) in shared.jobs.drain(..) {
            self.asked.remove(&key);
        }
        for (key, job) in jobs {
            if self.asked.insert(key) {
                shared.jobs.push_back((key, job));
            }
        }
        if !shared.jobs.is_empty() {
            wake.notify_one();
        }
    }

    /// Puts `key` as read (by `job`, into `read`) as the thread does.
    #[cfg(test)]
    pub(crate) fn put_read(&self, key: K, job: J, read: R) {
        if let Ok(mut shared) = self.shared.0.lock() {
            shared.read.push((key, job, read));
        }
    }

    /// Waits (up to `timeout`) until every job asked for is read and not taken; whether it came.
    #[cfg(test)]
    pub(crate) fn wait_read(&self, timeout: std::time::Duration) -> bool {
        let started = std::time::Instant::now();
        while started.elapsed() < timeout {
            if let Ok(shared) = self.shared.0.lock()
                && shared.jobs.is_empty()
                && shared.read.len() == self.asked.len()
            {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        false
    }

    /// The jobs read since the last call, with what was read.
    pub(crate) fn take(&mut self) -> Vec<(K, J, R)> {
        let Ok(mut shared) = self.shared.0.lock() else { return Vec::new() };
        let read = std::mem::take(&mut shared.read);
        for (key, _, _) in &read {
            self.asked.remove(key);
        }
        read
    }
}

impl<K, J, R> Drop for Reader<K, J, R> {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut shared) = lock.lock() {
            shared.closed = true;
            shared.jobs.clear();
        }
        wake.notify_all();
    }
}

/// The next job the reading thread reads, waiting for one; `None` once the reader is gone.
fn next_job<K, J, R>(shared: &Channel<K, J, R>) -> Option<(K, J)> {
    let (lock, wake) = shared;
    let mut shared = lock.lock().ok()?;
    loop {
        if shared.closed {
            return None;
        }
        if let Some(job) = shared.jobs.pop_front() {
            return Some(job);
        }
        shared = wake.wait(shared).ok()?;
    }
}
