//! Pure bounded admission state machine, also tested without native libraries.
pub const REQUESTS: usize = 12;
pub static INPUTS: [usize; REQUESTS] = [0, 1, 2, 0, 2, 1, 1, 0, 2, 2, 1, 0];
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    Queued,
    Running,
    Finished,
}
pub struct Schedule {
    states: [State; REQUESTS],
    capacity: usize,
    inflight: usize,
    completed: usize,
}
impl Schedule {
    pub const fn new(capacity: usize) -> Self {
        Self {
            states: [State::Pending; REQUESTS],
            capacity,
            inflight: 0,
            completed: 0,
        }
    }
    pub fn admit(&mut self) -> Option<usize> {
        if self.inflight == self.capacity {
            return None;
        }
        let index = self.states.iter().position(|s| *s == State::Pending)?;
        self.states[index] = State::Queued;
        self.inflight += 1;
        Some(index)
    }
    pub fn take(&mut self) -> Option<usize> {
        let index = self.states.iter().position(|s| *s == State::Queued)?;
        self.states[index] = State::Running;
        Some(index)
    }
    pub fn finish(&mut self, index: usize) -> Result<(), &'static str> {
        if self.states.get(index) != Some(&State::Running) {
            return Err("duplicate/unowned completion");
        }
        self.states[index] = State::Finished;
        self.inflight -= 1;
        self.completed += 1;
        Ok(())
    }
    pub const fn complete(&self) -> bool {
        self.completed == REQUESTS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_capacities_preserve_twelve_unique_admissions_and_completions() {
        for capacity in 1..=3 {
            let mut schedule = Schedule::new(capacity);
            let mut admitted = [false; REQUESTS];
            let mut completed = [false; REQUESTS];
            while !schedule.complete() {
                while let Some(i) = schedule.admit() {
                    assert!(!admitted[i]);
                    admitted[i] = true;
                }
                assert!(schedule.inflight <= capacity);
                let first = schedule.take().expect("queued owner");
                assert!(!completed[first]);
                completed[first] = true;
                schedule.finish(first).expect("owned completion");
            }
            assert!(admitted.into_iter().all(|x| x));
            assert!(completed.into_iter().all(|x| x));
        }
    }
    #[test]
    fn completed_slot_refills_while_peer_remains_live() {
        let mut schedule = Schedule::new(3);
        assert_eq!(schedule.admit(), Some(0));
        assert_eq!(schedule.admit(), Some(1));
        assert_eq!(schedule.admit(), Some(2));
        assert_eq!(schedule.admit(), None);
        assert_eq!(schedule.take(), Some(0));
        assert_eq!(schedule.take(), Some(1));
        schedule.finish(0).expect("complete first");
        assert_eq!(schedule.take(), Some(2));
        assert_eq!(schedule.admit(), Some(3));
        assert!(schedule.states[1] == State::Running);
    }
    #[test]
    fn duplicate_and_unowned_completions_do_not_change_accounting() {
        let mut schedule = Schedule::new(1);
        assert!(schedule.finish(0).is_err());
        assert_eq!(schedule.admit(), Some(0));
        assert!(schedule.finish(0).is_err());
        assert_eq!(schedule.take(), Some(0));
        schedule.finish(0).expect("owned");
        assert!(schedule.finish(0).is_err());
        assert!(schedule.finish(REQUESTS).is_err());
        assert_eq!(schedule.completed, 1);
        assert_eq!(schedule.inflight, 0);
    }
    #[test]
    fn schedule_is_the_frozen_twelve_fixture_order() {
        assert_eq!(INPUTS, [0, 1, 2, 0, 2, 1, 1, 0, 2, 2, 1, 0]);
    }
}
