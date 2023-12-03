use std::{collections::VecDeque, num::NonZeroUsize};

use crate::{Process, ProcessState, Scheduler, StopReason, SyscallResult};

use super::process_manager::{
    CurrentProcessMeta, ProcessEntry, ProcessInformation, ProcessManager, ProcessMeta,
};

pub struct Cfs(ProcessManager);

#[derive(Debug)]
pub struct CfsProcessMeta {
    inner: ProcessMeta,
    vruntime: usize,
}

impl CfsProcessMeta {
    fn new(scheduler: &ProcessManager, priority: i8) -> Self {
        Self {
            inner: ProcessMeta::new(scheduler, priority),
            vruntime: 0,
        }
    }

    fn alloc(scheduler: &ProcessManager, priority: i8) -> Box<dyn ProcessInformation + Send> {
        Box::new(Self::new(scheduler, priority))
    }
}

impl ProcessInformation for CfsProcessMeta {
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
        self.vruntime += time;
    }

    fn add_syscall(&mut self) {
        self.inner.add_syscall();
        self.vruntime += 1;
    }

    fn as_process(&self) -> &dyn Process {
        self
    }
}

impl Process for CfsProcessMeta {
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
        format!("vruntime={}", self.vruntime)
    }
}

impl PartialEq for CfsProcessMeta {
    fn eq(&self, other: &Self) -> bool {
        self.vruntime.eq(&other.vruntime)
    }
}

impl PartialOrd for CfsProcessMeta {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.vruntime.cmp(&other.vruntime))
    }
}

fn get_next_process(
    processes: &mut VecDeque<ProcessEntry>,
    current_process: &mut Option<CurrentProcessMeta>,
    timeslice_factor: &mut usize,
    timeslice: NonZeroUsize,
) {
    let process_number = processes.len();
    *timeslice_factor = process_number.max(1);

    if let Some(current) = current_process {
        *timeslice_factor += 1;
        if current.process.state() == ProcessState::Running {
            let x = timeslice.get() / *timeslice_factor;
            current.remaining_timeslice = current.remaining_timeslice.min(x);
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
                remaining_timeslice: timeslice.get() / *timeslice_factor,
                execution_cycles: 0,
                syscall_cycles: 0,
            });
            return;
        }
        waiting_processes.push_back(process);
    }

    *processes = waiting_processes;
}

impl Cfs {
    pub fn new(cpu_time: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self(ProcessManager::new(
            cpu_time,
            minimum_remaining_timeslice,
            Box::new(get_next_process),
            Box::new(CfsProcessMeta::alloc),
        ))
    }
}

impl Scheduler for Cfs {
    fn next(&mut self) -> crate::SchedulingDecision {
        self.0.next()
    }

    fn stop(&mut self, reason: StopReason) -> SyscallResult {
        self.0.handle_stop(reason)
    }

    fn list(&mut self) -> Vec<&dyn Process> {
        self.0
            .get_processes()
            .into_iter()
            .inspect(|x| print!("pid {:?} are {:?}; ", x.pid(), x.extra()))
            .map(|x| x.as_process())
            .inspect(|x| print!("pidu {:?} are {:?}; ", x.pid(), x.extra()))
            .collect()
    }
}
