use crate::{Process, ProcessState, Scheduler, SchedulingDecision, StopReason, SyscallResult};

use std::{collections::VecDeque, num::NonZeroUsize};

use super::process_manager::{CurrentProcessMeta, ProcessContainer, ProcessManager, ProcessMeta};

pub struct RoundRobin(ProcessManager);

impl ProcessContainer for VecDeque<ProcessMeta> {
    fn get_next_process(&mut self, timeslice: NonZeroUsize) -> Option<CurrentProcessMeta> {
        let mut waiting_processes = VecDeque::new();

        while let Some(mut process) = self.pop_front() {
            if process.state == ProcessState::Ready {
                process.state = ProcessState::Running;
                self.extend(waiting_processes);
                return Some(CurrentProcessMeta {
                    process,
                    remaining_timeslice: timeslice.get(),
                    execution_cycles: 0,
                    syscall_cycles: 0,
                });
            }
            waiting_processes.push_back(process);
        }

        *self = waiting_processes;
        None
    }

    fn push_back(&mut self, process: ProcessMeta) {
        self.push_back(process);
    }

    fn get_iterator<'a>(&'a self) -> Box<dyn ExactSizeIterator<Item = &'a ProcessMeta> + 'a> {
        Box::new(self.iter())
    }

    fn get_mut_iterator<'a>(
        &'a mut self,
    ) -> Box<dyn ExactSizeIterator<Item = &'a mut ProcessMeta> + 'a> {
        Box::new(self.iter_mut())
    }
}

impl RoundRobin {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        let container = Box::new(VecDeque::new());
        Self(ProcessManager::new(
            container,
            timeslice,
            minimum_remaining_timeslice,
        ))
    }
}

impl Scheduler for RoundRobin {
    fn next(&mut self) -> SchedulingDecision {
        self.0.next()
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
