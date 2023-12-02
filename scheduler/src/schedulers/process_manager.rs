use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
};

use crate::{
    Pid, Process, ProcessState, StopReason,
    Syscall::{Exit, Fork, Signal, Sleep, Wait},
    SyscallResult,
};

#[derive(Debug)]
pub struct ProcessMeta {
    pid: Pid,
    state: ProcessState,
    last_update: usize,
    priority: i8,
    /// Information about the process' run time
    /// Tota, Syscall, Execution
    timings: (usize, usize, usize),
}

impl ProcessMeta {
    fn new(pid: Pid, priority: i8, creation_time: usize) -> Self {
        ProcessMeta {
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

#[derive(Debug)]
/// Information about the process that is currently using the CPU.
pub struct CurrentProcessMeta {
    /// The current process.
    pub process: ProcessMeta,
    /// The cycles that the process has executed on this run.
    pub execution_cycles: usize,
    /// The cycles that the process has spent in syscalls on this run.
    pub syscall_cycles: usize,
    /// The time this process has left before it is preempted.
    pub remaining_timeslice: usize,
}
pub struct ProcessManager {
    /// The maximum timeslice that is given to each process.
    pub timeslice: NonZeroUsize,
    /// The minimum timeslice that a process must have left
    /// after a syscall in order to remain on the CPU.
    pub minimum_remaining_timeslice: usize,
    /// Wether the scheduler will panic on the next query.
    pub will_panic: bool,
    /// The maximum pid that has been assigned yet.
    pub max_pid: usize,
    pub processes: VecDeque<ProcessMeta>,
    pub sleeping_processes: VecDeque<(ProcessMeta, usize)>,
    pub waiting_processes: HashMap<usize, VecDeque<ProcessMeta>>,
    /// The process that is currently running (or None if there isn't one).
    pub current_process: Option<CurrentProcessMeta>,
    /// The number of clock cycles that have passed since the scheduler was started.
    pub clock: usize,
}

impl ProcessManager {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self {
            minimum_remaining_timeslice,
            processes: VecDeque::new(),
            sleeping_processes: VecDeque::new(),
            waiting_processes: HashMap::new(),
            current_process: None,
            will_panic: false,
            max_pid: 0,
            timeslice,
            clock: 0,
        }
    }

    pub fn get_processes(&self) -> Vec<&ProcessMeta> {
        self.processes
            .iter()
            .chain(self.sleeping_processes.iter().map(|(x, _)| x))
            .chain(self.waiting_processes.values().flatten())
            .collect()
    }

    /// Wake up all processes that finished waiting.
    pub fn wake_processes(&mut self) {
        let sleeping = self.sleeping_processes.drain(..);
        let (awaken, sleeping) = sleeping.partition(|(_, wake_time)| *wake_time <= self.clock);
        self.sleeping_processes = sleeping;

        let awaken = awaken.into_iter().map(|(process, _)| ProcessMeta {
            state: ProcessState::Ready,
            ..process
        });

        self.processes.extend(awaken);
    }

    pub fn get_next_process(&mut self) -> Option<CurrentProcessMeta> {
        let mut waiting_processes = VecDeque::new();

        while let Some(mut process) = self.processes.pop_front() {
            if process.state == ProcessState::Ready {
                process.state = ProcessState::Running;
                self.processes.extend(waiting_processes);
                return Some(CurrentProcessMeta {
                    process,
                    remaining_timeslice: self.timeslice.get(),
                    execution_cycles: 0,
                    syscall_cycles: 0,
                });
            }
            waiting_processes.push_back(process);
        }

        self.processes = waiting_processes;
        None
    }

    pub fn update_timings(&mut self) {
        let processes = self.processes.iter_mut();

        let sleeping_processes = self.sleeping_processes.iter_mut();
        let sleeping_processes = sleeping_processes.map(|(process, _)| process);

        let waiting_processes = self.waiting_processes.values_mut().flatten();

        for i in processes.chain(sleeping_processes).chain(waiting_processes) {
            let elapsed_time = self.clock - i.last_update;
            i.last_update = self.clock;

            i.timings.0 += elapsed_time;
            if i.state == ProcessState::Running {
                i.timings.2 += elapsed_time;
            }
        }
    }

    pub fn handle_stop(&mut self, reason: StopReason) -> SyscallResult {
        match reason {
            StopReason::Syscall { syscall, remaining } => {
                if let Some(current) = self.current_process.as_mut() {
                    self.clock += current.remaining_timeslice - remaining;
                    current.process.timings.0 += 1;
                    current.process.timings.1 += 1;
                    current.syscall_cycles += 1;
                }

                let syscall_result = match syscall {
                    Fork(priority) => {
                        self.max_pid += 1;
                        let new_process =
                            ProcessMeta::new(Pid::new(self.max_pid), priority, self.clock);
                        let new_pid = new_process.pid();
                        self.processes.push_back(new_process);

                        SyscallResult::Pid(new_pid)
                    }
                    Exit => {
                        if let Some(current) = self.current_process.take() {
                            // Killing `init` while other processes are
                            // running will result in a panic.
                            if current.process.pid() == 1
                                && (self.processes.len() != 0
                                    || self.sleeping_processes.len() != 0
                                    || self.waiting_processes.len() != 0)
                            {
                                self.will_panic = true;
                            }

                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                    Signal(event) => {
                        if let Some(waiting_processes) = self.waiting_processes.remove(&event) {
                            let waiting_processes =
                                waiting_processes.into_iter().map(|mut process| {
                                    process.state = ProcessState::Ready;
                                    process
                                });

                            self.processes.extend(waiting_processes);
                        }
                        SyscallResult::Success
                    }
                    Wait(event) => {
                        if let Some(mut current) = self.current_process.take() {
                            current.process.state = ProcessState::Waiting { event: Some(event) };

                            let execution_time = self.clock - current.process.last_update;
                            if execution_time > current.syscall_cycles {
                                current.execution_cycles += execution_time - current.syscall_cycles;
                            }
                            current.process.timings.0 += current.execution_cycles;
                            current.process.timings.2 += current.execution_cycles;
                            current.process.last_update = self.clock;
                            self.waiting_processes
                                .entry(event)
                                .or_insert_with(VecDeque::new)
                                .push_back(current.process);
                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                    Sleep(time) => {
                        if let Some(CurrentProcessMeta {
                            mut process,
                            mut execution_cycles,
                            syscall_cycles,
                            ..
                        }) = self.current_process.take()
                        {
                            process.state = ProcessState::Waiting { event: None };
                            let execution_time = self.clock - process.last_update;
                            if execution_time > syscall_cycles {
                                execution_cycles += execution_time - syscall_cycles;
                            }
                            process.timings.0 += execution_cycles;
                            process.timings.2 += execution_cycles;
                            process.last_update = self.clock;

                            self.sleeping_processes
                                .push_back((process, self.clock + time));
                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                };

                // Update the timer.
                if let Some(CurrentProcessMeta {
                    execution_cycles,
                    syscall_cycles,
                    remaining_timeslice,
                    process,
                    ..
                }) = self.current_process.as_mut()
                {
                    if process.state == ProcessState::Running {
                        let execution_time = *remaining_timeslice - remaining;
                        if execution_time > *syscall_cycles {
                            *execution_cycles += execution_time - *syscall_cycles;
                        }
                        *remaining_timeslice = remaining;
                        process.last_update = self.clock;
                    }
                }
                self.update_timings();

                if let Some(current) = self.current_process.as_mut() {
                    if current.remaining_timeslice < self.minimum_remaining_timeslice {
                        current.process.state = ProcessState::Ready;
                        current.process.timings.0 += current.execution_cycles;
                        current.process.timings.2 += current.execution_cycles;
                    }
                }
                syscall_result
            }
            StopReason::Expired => {
                if let Some(current) = self.current_process.as_mut() {
                    self.clock += current.remaining_timeslice;
                    current.process.state = ProcessState::Ready;
                    current.process.timings.0 += current.remaining_timeslice;
                    current.process.timings.2 += current.remaining_timeslice;
                    current.process.last_update = self.clock;

                    self.update_timings();

                    SyscallResult::Success
                } else {
                    SyscallResult::NoRunningProcess
                }
            }
        }
    }
}
