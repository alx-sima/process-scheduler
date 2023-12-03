use crate::{Process, ProcessState, Scheduler, SchedulingDecision, StopReason, SyscallResult};

use std::{collections::VecDeque, num::NonZeroUsize};

use super::process_manager::{CurrentProcessMeta, ProcessEntry, ProcessManager, ProcessMeta};

pub struct RoundRobin(ProcessManager);

fn get_next_process(
    processes: &mut VecDeque<ProcessEntry>,
    current_process: &mut Option<CurrentProcessMeta>,
    _timeslice_factor: &mut usize,
    timeslice: NonZeroUsize,
) {
    if let Some(current) = current_process {
        if current.process.state() == ProcessState::Running {
            return;
        }
    }

    if let Some(current) = current_process.take() {
        processes.push_back(current.process);
    }
    let mut waiting_processes = VecDeque::new();

    while let Some(mut process) = processes.pop_front() {
        if process.state() == ProcessState::Ready {
            process.set_state(ProcessState::Running);
            processes.extend(waiting_processes);

            *current_process = Some(CurrentProcessMeta {
                process,
                remaining_timeslice: timeslice.get(),
                execution_cycles: 0,
                syscall_cycles: 0,
            });
            return;
        }
        waiting_processes.push_back(process);
    }

    *processes = waiting_processes;
}
impl RoundRobin {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self(ProcessManager::new(
            timeslice,
            minimum_remaining_timeslice,
            Box::new(get_next_process),
            Box::new(ProcessMeta::alloc),
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
        self.0
            .get_processes()
            .iter()
            .map(|x| x.as_process())
            .collect()
    }
}
