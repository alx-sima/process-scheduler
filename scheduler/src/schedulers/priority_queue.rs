use std::{cmp::Ordering, collections::VecDeque, num::NonZeroUsize};

use crate::{Pid, Process, ProcessState, Scheduler, SchedulingDecision, StopReason};
use crate::schedulers::round_robin::ProcessMeta;

use super::process_manager::{CurrentProcessMeta, ProcessInformation, ProcessManager};

/// A scheduler implementing a Round Robin algorithm with priorities.
pub struct PriorityQueue(ProcessManager<PriorityProcessMeta>);

impl PriorityQueue {
    /// Creates a new Round Robin with priorities scheduler.
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self(ProcessManager::new(timeslice, minimum_remaining_timeslice))
    }
}

impl Scheduler for PriorityQueue {
    fn next(&mut self) -> SchedulingDecision {
        self.0.get_next_process()
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
        self.0.handle_process_stop(reason)
    }

    fn list(&mut self) -> Vec<&dyn Process> {
        self.0.get_processes().inspect(|x| print!("/ {} {} /", x.priority, x.priority())).map(|x| x.as_process()).collect()
    }
}

/// A representation of a process, accounting for priorities.
struct PriorityProcessMeta {
    /// Fields inherited from ProcessMeta.
    inner: ProcessMeta,
    /// The process' priority.
    priority: i8,
    /// The process' initial priority. The priority can't go above this value.
    max_priority: i8,
}

impl PriorityProcessMeta {
    /// Creates a new process with the given pid and priority.
    fn new(pid: Pid, priority: i8, creation_time: usize) -> Self {
        Self {
            inner: ProcessMeta::new(pid, priority, creation_time),
            max_priority: priority,
            priority,
        }
    }
}

impl ProcessInformation for PriorityProcessMeta {
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
        let mut next_process = None;

        for (index, process) in processes.iter().enumerate() {
            if let Some((_, max_proc)) = next_process {
                if process > max_proc && process.state() == ProcessState::Ready {
                    next_process = Some((index, process));
                }
            } else {
                next_process = Some((index, process));
            }
        }

        if let Some((index, _)) = next_process {
            if let Some(mut process) = processes.remove(index) {
                process.set_state(ProcessState::Running);
                *current_process = Some(CurrentProcessMeta {
                    process,
                    execution_cycles: 0,
                    syscall_cycles: 0,
                    remaining_timeslice: timeslice.get(),
                });
            }
        }
    }

    fn as_process(&self) -> &dyn Process {
        self
    }

    fn set_state(&mut self, state: ProcessState) {
        self.inner.set_state(state);
    }

    fn last_update(&self) -> usize {
        self.inner.last_update()
    }

    fn set_last_update(&mut self, last_update: usize) {
        self.inner.set_last_update(last_update);
    }

    fn add_total_time(&mut self, time: usize) {
        self.inner.add_total_time(time);
    }

    fn add_execution_time(&mut self, time: usize) {
        self.inner.add_execution_time(time);
    }

    fn add_syscall(&mut self) {
        self.inner.add_syscall();
    }

    fn increase_priority(&mut self) {
        self.priority += 1;
        self.priority = self.priority.min(self.max_priority);
    }
    fn decrease_priority(&mut self) {
        self.priority -= 1;
        self.priority = self.priority.max(0);
    }
}

impl Process for PriorityProcessMeta {
    fn pid(&self) -> Pid {
        self.inner.pid()
    }

    fn state(&self) -> ProcessState {
        self.inner.state()
    }

    fn timings(&self) -> (usize, usize, usize) {
        self.inner.timings()
    }

    fn priority(&self) -> i8 {
        self.priority
    }

    fn extra(&self) -> String {
        String::new()
    }
}

impl PartialEq for PriorityProcessMeta {
    fn eq(&self, other: &Self) -> bool {
        self.priority().eq(&other.priority())
    }
}

impl PartialOrd for PriorityProcessMeta {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.priority().partial_cmp(&other.priority())
    }
}
