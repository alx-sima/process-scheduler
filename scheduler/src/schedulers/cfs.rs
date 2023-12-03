use std::{cmp::Ordering, collections::VecDeque, num::NonZeroUsize};
use std::ops::{Add, AddAssign};

use crate::{Pid, Process, ProcessState, Scheduler, StopReason, SyscallResult};
use crate::schedulers::round_robin::ProcessMeta;

use super::process_manager::{CurrentProcessMeta, ProcessInformation, ProcessManager};

/// A scheduler implementing a simplified version of avCompletely Fair Scheduler algorithm.
pub struct Cfs(ProcessManager<CfsProcessMeta>);

/// A representation of a process, accounting for virtual runtime.
struct CfsProcessMeta {
    /// Fields inherited from ProcessMeta.
    inner: ProcessMeta,
    /// The process' virtual runtime.
    vruntime: usize,
}

impl Cfs {
    /// Creates a new Completely Fair Scheduler.
    pub fn new(cpu_time: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self(ProcessManager::new(cpu_time, minimum_remaining_timeslice))
    }
}

impl Scheduler for Cfs {
    fn next(&mut self) -> crate::SchedulingDecision {
        self.0.get_next_process()
    }

    fn stop(&mut self, reason: StopReason) -> SyscallResult {
        self.0.handle_process_stop(reason)
    }

    fn list(&mut self) -> Vec<&dyn Process> {
        self.0.get_processes().map(|x| x.as_process()).collect()
    }
}

impl CfsProcessMeta {
    fn new(pid: Pid, priority: i8, creation_time: usize, vruntime: usize) -> Self {
        Self {
            inner: ProcessMeta::new(pid, priority, creation_time),
            vruntime,
        }
    }
}

impl ProcessInformation for CfsProcessMeta {
    fn create_process(scheduler: &ProcessManager<Self>, priority: i8) -> Self {
        let vruntime = scheduler
            .get_processes()
            .map(|x| x.vruntime())
            .min()
            .unwrap_or_default();

        Self::new(
            Pid::new(scheduler.max_pid),
            priority,
            scheduler.clock,
            vruntime,
        )
    }

    fn next_process(
        processes: &mut VecDeque<Self>,
        current_process: &mut Option<CurrentProcessMeta<Self>>,
        timeslice_factor: &mut usize,
        timeslice: NonZeroUsize,
    ) {
        *timeslice_factor = processes.len();

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

        let mut next_process: Option<(usize, &CfsProcessMeta)> = None;
        for (index, process) in processes.iter().enumerate() {
            if process.state() != ProcessState::Ready {
                continue;
            }
            if let Some((_, min_proc)) = next_process {
                if process < min_proc {
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
                    remaining_timeslice: timeslice.get() / *timeslice_factor,
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
        *self += time;
    }

    fn add_syscall(&mut self) {
        self.inner.add_syscall();
        *self += 1;
    }

    fn vruntime(&self) -> usize {
        self.vruntime
    }
}

impl Process for CfsProcessMeta {
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
        self.inner.priority()
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
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.vruntime.partial_cmp(&other.vruntime) {
            Some(Ordering::Equal) => self.inner.pid().partial_cmp(&other.inner.pid()),
            x => x,
        }
    }
}

impl Add<usize> for CfsProcessMeta {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self {
            inner: self.inner,
            vruntime: self.vruntime + rhs,
        }
    }
}

impl AddAssign<usize> for CfsProcessMeta {
    fn add_assign(&mut self, rhs: usize) {
        self.vruntime += rhs;
    }
}
