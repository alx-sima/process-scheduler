use crate::{
    Pid, Process, ProcessState, Scheduler, StopReason,
    Syscall::{Exit, Fork, Signal, Sleep, Wait},
    SyscallResult,
};

use std::num::NonZeroUsize;

struct ProcessMeta {
    pid: Pid,
}

impl Process for ProcessMeta {
    fn pid(&self) -> Pid {
        self.pid
    }

    fn state(&self) -> ProcessState {
        todo!()
    }

    fn timings(&self) -> (usize, usize, usize) {
        todo!()
    }

    fn priority(&self) -> i8 {
        todo!()
    }

    fn extra(&self) -> String {
        todo!()
    }
}

pub struct RoundRobin {
    timeslice: NonZeroUsize,
    minimum_remaining_timeslice: usize,
    processes: Vec<ProcessMeta>,
    current_process: usize,
}

impl RoundRobin {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        RoundRobin {
            timeslice,
            minimum_remaining_timeslice,
            processes: Vec::new(),
            current_process: 0,
        }
    }
}

impl Scheduler for RoundRobin {
    fn next(&mut self) -> crate::SchedulingDecision {
        unimplemented!()
    }

    fn stop(&mut self, reason: StopReason) -> crate::SyscallResult {
        match reason {
            StopReason::Syscall {
                syscall,
                remaining: _,
            } => match syscall {
                Fork(_) => {
                    let new_pid = Pid::new(self.processes.len());
                    self.processes.push(ProcessMeta { pid: new_pid });

                    SyscallResult::Pid(new_pid)
                }
                Exit => todo!(),
                Signal(_) => todo!(),
                Wait(_) => todo!(),
                Sleep(_) => todo!(),
            },
            StopReason::Expired => todo!(),
        }
    }

    fn list(&mut self) -> Vec<&dyn Process> {
        self.processes.iter().map(|p| p as &dyn Process).collect()
    }
}
