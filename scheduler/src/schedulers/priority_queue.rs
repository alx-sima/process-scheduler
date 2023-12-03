use std::{cmp::Ordering, collections::VecDeque, num::NonZeroUsize};

use crate::{Process, ProcessState, Scheduler, SchedulingDecision, StopReason};

use super::process_manager::{
    CurrentProcessMeta, ProcessEntry, ProcessInformation, ProcessManager, ProcessMeta,
};

struct PriorityProcessMeta {
    inner: ProcessMeta,
    max_priority: i8,
}

impl PriorityProcessMeta {
    fn new(scheduler: &ProcessManager, priority: i8) -> Self {
        Self {
            inner: ProcessMeta::new(scheduler, priority),
            max_priority: priority,
        }
    }

    fn alloc(scheduler: &ProcessManager, priority: i8) -> Box<dyn ProcessInformation + Send> {
        Box::new(Self::new(scheduler, priority))
    }
}

impl ProcessInformation for PriorityProcessMeta {
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

    fn as_process(&self) -> &dyn Process {
        self.inner.as_process()
    }

    fn increase_priority(&mut self) {
        self.inner.priority += 1;
        self.inner.priority = self.inner.priority.min(self.max_priority);
    }

    fn decrease_priority(&mut self) {
        self.inner.priority -= 1;
        self.inner.priority = self.inner.priority.max(0);
    }
}

impl Process for PriorityProcessMeta {
    fn pid(&self) -> crate::Pid {
        self.inner.pid()
    }

    fn state(&self) -> ProcessState {
        self.inner.state()
    }

    fn priority(&self) -> i8 {
        self.inner.priority()
    }

    fn timings(&self) -> (usize, usize, usize) {
        self.inner.timings()
    }

    fn extra(&self) -> String {
        String::new()
    }
}

pub struct PriorityQueue(ProcessManager);

impl PartialEq for &(dyn ProcessInformation + Send) {
    fn eq(&self, other: &Self) -> bool {
        self.priority().eq(&other.priority())
    }
}

impl PartialOrd for &(dyn ProcessInformation + Send) {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.priority().partial_cmp(&other.priority())
    }
}

impl PriorityQueue {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self(ProcessManager::new(
            timeslice,
            minimum_remaining_timeslice,
            Box::new(get_next_process),
            Box::new(PriorityProcessMeta::alloc),
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
        self.0
            .get_processes()
            .iter()
            .map(|x| x.as_process())
            .collect()
    }
}
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
    let mut next_process: Option<(usize, &ProcessEntry)> = None;

    for (index, process) in processes.iter().enumerate() {
        if let Some((_, max_proc)) = next_process {
            if process.as_ref() > max_proc.as_ref() && process.state() == ProcessState::Ready {
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
