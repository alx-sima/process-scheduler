use std::num::NonZeroUsize;

use crate::{Process, ProcessState, Scheduler, SchedulingDecision, StopReason};

use super::process_manager::{CurrentProcessMeta, ProcessContainer, ProcessManager, ProcessMeta};

pub struct PriorityQueue(ProcessManager);

impl PartialEq for ProcessMeta {
    fn eq(&self, other: &Self) -> bool {
        self.priority().eq(&other.priority())
    }
}

impl PartialOrd for ProcessMeta {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.priority().partial_cmp(&other.priority())
    }
}

impl PriorityQueue {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        let container = Box::new(Vec::new());
        Self(ProcessManager::new(
            container,
            timeslice,
            minimum_remaining_timeslice,
        ))
    }
}

impl Scheduler for PriorityQueue {
    fn next(&mut self) -> SchedulingDecision {
        self.0.next()
    }

    fn stop(&mut self, reason: StopReason) -> crate::SyscallResult {
        if let Some(current_process) = self.0.current_process.as_mut() {
            match reason {
                StopReason::Syscall {
                    syscall: _,
                    remaining: _,
                } => current_process.process.increase_priority(),
                StopReason::Expired => {
                    current_process.process.decrease_priority();
                }
            }
        }
        self.0.handle_stop(reason)
    }

    fn list(&mut self) -> Vec<&dyn crate::Process> {
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

impl ProcessContainer for Vec<ProcessMeta> {
    fn get_next_process(&mut self, timeslice: NonZeroUsize) -> Option<CurrentProcessMeta> {
        let mut next_process: Option<(usize, &ProcessMeta)> = None;

        for (index, process) in self.get_iterator().enumerate() {
            if let Some((_, max_proc)) = next_process {
                if *process > *max_proc && process.state() == ProcessState::Ready {
                    next_process = Some((index, process));
                }
            } else {
                next_process = Some((index, process));
            }
        }

        if let Some((index, _)) = next_process {
            let mut process = self.remove(index);
            process.state = ProcessState::Running;
            return Some(CurrentProcessMeta {
                process,
                execution_cycles: 0,
                syscall_cycles: 0,
                remaining_timeslice: timeslice.get(),
            });
        }
        None
    }

    fn push_back(&mut self, process: ProcessMeta) {
        self.push(process);
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
