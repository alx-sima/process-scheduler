use crate::{Process, ProcessState, Scheduler, SchedulingDecision, StopReason, SyscallResult};

use std::num::NonZeroUsize;

use super::process_manager::ProcessManager;

pub struct RoundRobin(ProcessManager);

impl RoundRobin {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self(ProcessManager::new(timeslice, minimum_remaining_timeslice))
    }
}

impl Scheduler for RoundRobin {
    fn next(&mut self) -> SchedulingDecision {
        self.0.update_timings();
        self.0.wake_processes();

        if self.0.will_panic {
            return SchedulingDecision::Panic;
        }

        if let Some(current) = &self.0.current_process {
            if current.process.state() == ProcessState::Running {
                return SchedulingDecision::Run {
                    pid: current.process.pid(),
                    timeslice: NonZeroUsize::new(current.remaining_timeslice).unwrap(),
                };
            }
        }

        if let Some(current) = self.0.current_process.take() {
            self.0.processes.push_back(current.process);
        }

        self.0.current_process = self.0.get_next_process();

        if let Some(current) = &self.0.current_process {
            return SchedulingDecision::Run {
                pid: current.process.pid(),
                timeslice: NonZeroUsize::new(current.remaining_timeslice).unwrap(),
            };
        }

        // There aren't any ready processes, wait for the first to wake up.
        if let Some(first_wake) = self
            .0
            .sleeping_processes
            .iter()
            .map(|(_, wake_time)| wake_time)
            .min()
        {
            let wait_interval = first_wake - self.0.clock;
            self.0.clock = *first_wake;

            return SchedulingDecision::Sleep(NonZeroUsize::new(wait_interval).unwrap());
        } else {
            // There are no processes to be awaken. If there still
            // are processes waiting, signal a deadlock.
            if self.0.processes.len() != 0 || self.0.waiting_processes.len() != 0 {
                return SchedulingDecision::Deadlock;
            } else {
                return SchedulingDecision::Done;
            }
        }
    }

    fn stop(&mut self, reason: StopReason) -> SyscallResult {
        self.0.handle_stop(reason)
    }

    fn list(&mut self) -> Vec<&dyn Process> {
        let mut processes = Vec::new();

        if let Some(current) = &self.0.current_process {
            processes.push(&current.process as &dyn Process);
        }

        processes.extend(
            self.0
                .get_processes()
                .into_iter()
                .map(|x| x as &dyn Process),
        );
        processes
    }
}
