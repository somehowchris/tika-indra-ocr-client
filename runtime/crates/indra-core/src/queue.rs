//! Bounded FIFO storage. The caller supplies synchronization and wakeups.

#[derive(Debug)]
pub struct Queue<T, const N: usize> {
    entries: [Option<T>; N],
    head: usize,
    len: usize,
}

impl<T, const N: usize> Default for Queue<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Queue<T, N> {
    pub const fn new() -> Self {
        Self {
            entries: [const { None }; N],
            head: 0,
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    /// Full queues return ownership of the rejected value.
    pub fn push(&mut self, value: T) -> Result<(), T> {
        if self.len == N {
            return Err(value);
        }
        let tail = (self.head + self.len) % N;
        debug_assert!(self.entries[tail].is_none());
        self.entries[tail] = Some(value);
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.entries[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        value
    }

    pub fn front(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            self.entries[self.head].as_ref()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn wraps_preserving_fifo_and_returns_rejected_ownership() {
        let mut queue = Queue::<u32, 2>::new();
        assert_eq!(queue.push(1), Ok(()));
        assert_eq!(queue.push(2), Ok(()));
        assert_eq!(queue.push(3), Err(3));
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.push(3), Ok(()));
        assert_eq!(queue.front(), Some(&2));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn zero_capacity_is_safe_and_never_divides_by_zero() {
        let mut queue = Queue::<u8, 0>::new();
        assert_eq!(queue.push(7), Err(7));
        assert_eq!(queue.pop(), None);
        assert_eq!(queue.front(), None);
    }

    #[test]
    fn retained_items_are_dropped_exactly_once() {
        struct CountDrop<'a>(&'a AtomicUsize);
        impl Drop for CountDrop<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let dropped = AtomicUsize::new(0);
        {
            let mut queue = Queue::<CountDrop<'_>, 2>::new();
            assert!(queue.push(CountDrop(&dropped)).is_ok());
            assert!(queue.push(CountDrop(&dropped)).is_ok());
            let rejected = queue
                .push(CountDrop(&dropped))
                .expect_err("full queue returns ownership");
            assert_eq!(dropped.load(Ordering::Relaxed), 0);
            drop(queue.pop());
            assert_eq!(dropped.load(Ordering::Relaxed), 1);
            drop(rejected);
            assert_eq!(dropped.load(Ordering::Relaxed), 2);
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 3);
    }
}
