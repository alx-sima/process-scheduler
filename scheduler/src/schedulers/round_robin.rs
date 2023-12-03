use std::collections::VecDeque;
use crate::{Pid, Process, ProcessState, Scheduler, SchedulingDecision, StopReason, SyscallResult};

use std::num::NonZeroUsize;

use super::process_manager::{ProcessManager, ProcessInformation, CurrentProcessMeta};

/// A scheduler implementing a Round Robin algorithm.
pub struct RoundRobin(ProcessManager<ProcessMeta>);

impl RoundRobin {
    /// Creates a new Round Robin scheduler.
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

/// A simple representation of a process.
pub(super) struct ProcessMeta {
    /// The process' pid.
    pid: Pid,
    /// The process' state.
    state: ProcessState,
    /// When was this process last updated.
    last_update: usize,
    /// The process' priority (not used in scheduling).
    priority: i8,
    /// Information about the process' run time.
    /// (Total, Syscall, Execution)
    timings: (usize, usize, usize),
}

impl ProcessMeta {
    /// Creates a new process with the given pid and priority.
    pub fn new(pid: Pid, priority: i8, creation_time: usize) -> Self {
        Self {
            pid,
            priority,
            last_update: creation_time,
            state: ProcessState::Ready,
            timings: (0, 0, 0),
        }
    }
}

impl Process for ProcessMeta {
    fn pid(&self) -> Pid {
        self.pid
    }

    fn state(&self) -> ProcessState {
        self.state
    }

    fn timings(&self) -> (usize, usize, usize) {
        self.timings
    }

    fn priority(&self) -> i8 {
        self.priority
    }

    fn extra(&self) -> String {
        String::new()
    }
}

impl ProcessInformation for ProcessMeta {
    fn create_process(scheduler: &ProcessManager<Self>, priority: i8) -> Self {
        Self::new(Pid::new(scheduler.max_pid), priority, scheduler.clock)
    }

    fn next_process(
        processes: &mut VecDeque<Self>,
        current_process: &mut Option<CurrentProcessMeta<Self>>,
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

    fn as_process(&self) -> &dyn Process {
        self
    }

    fn set_state(&mut self, state: ProcessState) {
        self.state = state;
    }

    fn last_update(&self) -> usize {
        self.last_update
    }

    fn set_last_update(&mut self, last_update: usize) {
        self.last_update = last_update;
    }

    fn add_total_time(&mut self, time: usize) {
        self.timings.0 += time;
    }
    fn add_execution_time(&mut self, time: usize) {
        self.timings.0 += time;
        self.timings.2 += time;
    }

    fn add_syscall(&mut self) {
        self.timings.0 += 1;
        self.timings.1 += 1;
    }
}