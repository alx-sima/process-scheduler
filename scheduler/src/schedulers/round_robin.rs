use crate::{
    Pid, Process, ProcessState, Scheduler, SchedulingDecision, StopReason,
    Syscall::{Exit, Fork, Signal, Sleep, Wait},
    SyscallResult,
};

use std::{collections::VecDeque, num::NonZeroUsize};

struct ProcessMeta {
    pid: Pid,
    state: ProcessState,
    timings: (
        // Total
        usize,
        // Syscall
        usize,
        // Execution
        usize),
}

impl ProcessMeta {
    fn new(pid: Pid) -> Self {
        ProcessMeta {
            pid,
            state: ProcessState::Running,
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
        0
    }

    fn extra(&self) -> String {
        String::new()
    }
}

struct SleepScheduler {
    timer: usize,
    sleeping_processes: VecDeque<(ProcessMeta, usize)>,
}

pub struct RoundRobin {
    timeslice: NonZeroUsize,
    minimum_remaining_timeslice: usize,
    processes: VecDeque<ProcessMeta>,
    sleep_timer: SleepScheduler,
}

impl RoundRobin {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        RoundRobin {
            timeslice,
            minimum_remaining_timeslice,
            processes: VecDeque::new(),
            sleep_timer: SleepScheduler { timer: 0usize, sleeping_processes: VecDeque::new() },
        }
    }
}

impl Scheduler for RoundRobin {
    fn next(&mut self) -> SchedulingDecision {
        if self.processes.is_empty() {
            return SchedulingDecision::Done;
        }

        for process in self.processes.iter_mut() {
            if process.state == ProcessState::Ready {
                process.state = ProcessState::Running;
                return SchedulingDecision::Run {
                    pid: process.pid(),
                    timeslice: self.timeslice,
                };
            }
        }

        SchedulingDecision::Deadlock
    }

    fn stop(&mut self, reason: StopReason) -> SyscallResult {
        match reason {
            StopReason::Syscall {
                syscall,
                remaining,
            } =>
                {
                    // Update the sleep timer.
                    let time_passed = self.timeslice.get() - remaining;
                    self.sleep_timer.timer += time_passed;

                    match syscall {
                        Fork(_) => {
                            let new_pid = Pid::new(self.processes.len() + 1);
                            self.processes.push_back(ProcessMeta::new(new_pid));

                            SyscallResult::Pid(new_pid)
                        }
                        Exit => {
                            if self.processes.pop_front().is_some() {
                                SyscallResult::Success
                            } else {
                                SyscallResult::NoRunningProcess
                            }
                        }
                        Signal(event) => {
                            for proc in &mut self.processes {
                                if let ProcessState::Waiting { event: Some(proc_event) } = proc.state {
                                    if proc_event == event {
                                        proc.state = ProcessState::Ready;
                                    }
                                }
                            }

                            SyscallResult::Success
                        }
                        Wait(event) => {
                            if let Some(mut process) = self.processes.pop_front() {
                                process.state = ProcessState::Waiting { event: Some(event) };
                                process.timings.0 += self.timeslice.get() - remaining;

                                self.processes.push_back(process);
                                SyscallResult::Success
                            } else {
                                SyscallResult::NoRunningProcess
                            }
                        }
                        Sleep(time) => {
                            if let Some(mut process) = self.processes.pop_front() {
                                process.state = ProcessState::Waiting { event: None };
                                process.timings.0 += self.timeslice.get() - remaining;

                                self.sleep_timer.sleeping_processes.push_back((process, time));
                                SyscallResult::Success
                            } else {
                                SyscallResult::NoRunningProcess
                            }
                        }
                    }
                }
            StopReason::Expired => {
                if let Some(mut process) = self.processes.pop_front() {
                    if process.state == ProcessState::Running {
                        self.sleep_timer.timer += self.timeslice.get();
                        process.timings.0 += self.timeslice.get();
                        self.processes.push_back(process);
                    }
                }

                SyscallResult::Success
            }
        }
    }

    fn list(&mut self) -> Vec<&dyn Process> {
        self.processes.iter().map(|p| p as &dyn Process).collect()
    }
}
