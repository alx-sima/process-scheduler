use crate::{Process, Scheduler, SchedulingDecision, StopReason, SyscallResult};

use std::num::NonZeroUsize;

use super::process_manager::{ProcessManager, ProcessMeta, ProcessInformation};

pub struct RoundRobin(ProcessManager<ProcessMeta>);

impl RoundRobin {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self(ProcessManager::new(
            timeslice,
            minimum_remaining_timeslice,
        ))
    }
}

impl Scheduler for RoundRobin {
    fn next(&mut self) -> SchedulingDecision {
        self.0.get_next_process()
    }

    fn stop(&mut self, reason: StopReason) -> SyscallResult {
        self.0.handle_process_stop(reason)
    }

    fn list(&mut self) -> Vec<&dyn Process> {
        self.0.get_processes().map(|x| x.as_process()).collect()
    }
}
